mod client;
mod error;

use std::{
    fs::read_to_string,
    io::{stdout, Stdout, Write},
};

use clap::Parser;

extern crate clipboard;

use clipboard::{ClipboardContext, ClipboardProvider};

use client::VaultSecret;
use console::{style, Key, Term};
use home::home_dir;
use termion::screen::{AlternateScreen, IntoAlternateScreen};

use crate::client::VaultClient;

#[derive(Clone)]
struct VaultEntry {
    name: String,
    is_dir: bool,
}

impl VaultEntry {
    fn decode(name: &str) -> VaultEntry {
        let is_dir = name.ends_with('/');
        VaultEntry {
            name: if is_dir {
                name[..name.len() - 1].to_owned()
            } else {
                name.to_owned()
            },
            is_dir,
        }
    }
}

struct VaultPath {
    entries: Vec<VaultEntry>,
}

impl VaultPath {
    fn join(&self) -> String {
        self.entries.iter().fold(String::new(), |acc, item| {
            acc + &item.name + if item.is_dir { "/" } else { "" }
        })
    }

    fn len(&self) -> usize {
        self.entries
            .iter()
            .fold(0, |acc, item| acc + item.name.len() + 1)
    }

    fn decode(path: &str) -> VaultPath {
        VaultPath {
            entries: path
                .split_inclusive('/')
                .filter(|x| !x.is_empty())
                .map(VaultEntry::decode)
                .collect(),
        }
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    root_path: String,
    #[arg(
        short = 'H',
        long,
        help = "URL of the vault server, defaults to $VAULT_ADDR"
    )]
    host: Option<String>,
    #[arg(
        short,
        long,
        help = "Vault token, default to the value in ~/.vault-token"
    )]
    token: Option<String>,
}

#[derive(PartialEq, Copy, Clone)]
enum SecretEdition {
    Insert,
    Update,
}

#[derive(PartialEq)]
enum Mode {
    Navigation,
    TypingKey,
    TypingSecret(SecretEdition),
    DeletingKey,
}

struct Vaultwalker {
    client: VaultClient,
    screen: Option<AlternateScreen<Stdout>>,
    term: Term,
    clipboard: ClipboardContext,
    mode: Mode,
    quit_requested: bool,
    path: VaultPath,
    root_len: usize,
    current_list: Vec<VaultEntry>,
    selected_item: usize,
    selected_secret: Option<VaultSecret>,
    displayed_message: String,
    buffered_key: String,
}

impl Vaultwalker {
    fn new(host: String, token: String, root: String, use_alternate_screen: bool) -> Self {
        let path = VaultPath::decode(&root);
        Self {
            client: VaultClient::new(&host, &token),
            screen: if use_alternate_screen {
                Some(stdout().into_alternate_screen().unwrap())
            } else {
                None
            },
            term: Term::stdout(),
            clipboard: ClipboardProvider::new().unwrap(),
            mode: Mode::Navigation,
            quit_requested: false,
            root_len: path.entries.len(),
            path,
            current_list: vec![],
            selected_item: 0,
            selected_secret: None,
            displayed_message: String::new(),
            buffered_key: String::new(),
        }
    }

    fn setup(&mut self) {
        self.term.hide_cursor().unwrap();
        self.update_list(true);
        self.print();
        self.print_controls();
        if let Some(screen) = self.screen.as_mut() {
            screen.flush().unwrap();
        }
    }

    fn update_list(&mut self, no_cache: bool) {
        let path = self.path.join();
        let res = self.client.list_secrets(&path, no_cache).unwrap();
        self.current_list = res.keys.iter().map(|x| VaultEntry::decode(x)).collect();
    }

    fn update_selected_secret(&mut self, no_cache: bool) {
        if self.current_list[self.selected_item].is_dir {
            self.selected_secret = None;
            return;
        }
        let mut path = self.path.join();
        path.push_str(&self.current_list[self.selected_item].name);
        let res = self.client.get_secret(&path, no_cache).unwrap();
        self.selected_secret = Some(res);
    }

    fn refresh_all(&mut self) {
        self.update_list(true);
        self.update_selected_secret(true);
    }

    fn selected_line_for_current_mode(&self, item: &VaultEntry) -> String {
        match self.mode {
            Mode::Navigation | Mode::DeletingKey => {
                let mut line = format!("> {}{}", item.name, if item.is_dir { "/" } else { "" });

                if let Some(secret) = self.selected_secret.as_ref() {
                    if secret.secret.is_some() {
                        line.push_str(&format!(
                            " -> {}",
                            &style(&secret.secret.as_ref().unwrap()).bold().bright()
                        ));
                    }
                }

                line
            }
            Mode::TypingKey => format!("> {}{}", item.name, if item.is_dir { "/" } else { "" }),
            Mode::TypingSecret(_) => {
                format!("> {}{} -> ", item.name, if item.is_dir { "/" } else { "" })
            }
        }
    }

    fn print(&mut self) {
        self.term.clear_screen().unwrap();

        let mut extended_item = Vec::new();
        match self.mode {
            Mode::Navigation | Mode::DeletingKey => (),
            Mode::TypingKey => extended_item.push(VaultEntry {
                name: self.buffered_key.clone(),
                is_dir: false,
            }),
            Mode::TypingSecret(_) => extended_item.push(VaultEntry {
                name: self.buffered_key.clone(),
                is_dir: false,
            }),
        }

        let mut len_selected = 0;
        let prefix_len = self.path.len() + 1;
        for (i, item) in self
            .current_list
            .iter()
            .chain(extended_item.iter())
            .enumerate()
        {
            let mut line = String::new();
            if i == 0 {
                line.push_str(&format!("{} ", &style(self.path.join()).bold().bright()));
            } else {
                line.push_str(&format!("{:prefix$}", "", prefix = prefix_len));
            }

            if i == self.selected_item {
                line.push_str(&self.selected_line_for_current_mode(item));
                len_selected = line.len();
            } else {
                line.push_str(&format!(
                    "  {}{}",
                    item.name,
                    if item.is_dir { "/" } else { "" }
                ))
            }

            if i < self.current_list.len() {
                line.push('\n')
            }

            self.term.write_all(line.as_bytes()).unwrap();
        }

        match self.mode {
            Mode::TypingKey | Mode::TypingSecret(_) => self
                .term
                .move_cursor_to(len_selected, self.selected_item)
                .unwrap(),
            _ => (),
        }
    }

    fn print_message(&mut self, message: &str) {
        if self.displayed_message == message {
            return;
        }

        self.displayed_message = message.to_owned();
        self.term.move_cursor_down(10000).unwrap();
        self.term.clear_line().unwrap();
        self.term
            .write_all(style(message).black().on_white().to_string().as_bytes())
            .unwrap();
    }

    fn print_controls(&mut self) {
        self.print_message(
            "navigate with arrows      P: copy path      S: copy secret      A: create secret      U: update secret      D: delete secret      Q: quit      C: clear cache",
        );
    }

    fn handle_navigation(&mut self) {
        let mut needs_refresh = false;
        match self.term.read_key().unwrap() {
            Key::ArrowDown | Key::Char('j') => {
                if self.selected_item < self.current_list.len() - 1 {
                    self.selected_item += 1;
                }
                needs_refresh = true;
            }
            Key::ArrowUp | Key::Char('k') => {
                if self.selected_item > 0 {
                    self.selected_item -= 1;
                }
                needs_refresh = true;
            }
            Key::ArrowRight | Key::Char('l') => {
                if self.path.entries.len() > 32 {
                    return;
                }
                let entry = self.current_list[self.selected_item].clone();
                if !entry.is_dir {
                    return;
                }
                self.path.entries.push(entry);
                self.update_list(false);
                self.selected_item = self.selected_item.min(self.current_list.len() - 1);
                needs_refresh = true;
            }
            Key::ArrowLeft | Key::Char('h') => {
                if self.path.entries.len() < self.root_len + 1 {
                    return;
                }
                let last = self.path.entries.pop().unwrap();
                self.update_list(false);
                self.selected_item = self
                    .current_list
                    .iter()
                    .position(|x| x.name == last.name)
                    .unwrap();
                needs_refresh = true;
            }
            Key::Char('c') => {
                self.client.clear_cache();
                self.update_list(false);
                self.update_selected_secret(false);

                needs_refresh = true;
            }
            Key::Char('p') => {
                let mut path = self.path.join();
                path.push_str(&self.current_list[self.selected_item].name);
                self.clipboard.set_contents(path).unwrap();

                self.print_message("path copied to clipboard");
            }
            Key::Char('s') => {
                let entry = self.current_list[self.selected_item].clone();
                if entry.is_dir {
                    return;
                }

                if let Some(secret) = self.selected_secret.as_ref() {
                    if let Some(secret) = secret.secret.as_ref() {
                        self.clipboard.set_contents(secret.clone()).unwrap();

                        self.print_message("secret copied to clipboard");
                    }
                }
            }
            Key::Char('a') => {
                self.selected_item = self.current_list.len();
                self.term.show_cursor().unwrap();
                self.mode = Mode::TypingKey;

                needs_refresh = true;
            }
            Key::Char('u') => {
                let entry = self.current_list[self.selected_item].clone();
                if entry.is_dir {
                    return;
                }

                self.term.show_cursor().unwrap();
                self.mode = Mode::TypingSecret(SecretEdition::Update);

                needs_refresh = true;
            }
            Key::Char('d') => {
                self.mode = Mode::DeletingKey;

                needs_refresh = true;
            }
            Key::Escape | Key::Char('q') => self.quit_requested = true,
            _ => (),
        }

        if needs_refresh {
            if self.mode == Mode::Navigation {
                self.update_selected_secret(false);
            }
            self.print();
        }
    }

    fn handle_typing_key(&mut self) {
        self.buffered_key = self.term.read_line().unwrap();
        self.mode = Mode::TypingSecret(SecretEdition::Insert);

        self.print();
    }

    fn handle_typing_secret(&mut self, secret_type: SecretEdition) {
        let secret = self.term.read_line().unwrap();
        self.mode = Mode::Navigation;

        let path = match secret_type {
            SecretEdition::Insert => {
                let mut path = self.path.join();
                path.push_str(&self.buffered_key);
                path
            }
            SecretEdition::Update => {
                let mut path = self.path.join();
                path.push_str(&self.current_list[self.selected_item].name);
                path
            }
        };

        let res = self.client.write_secret(&path, &secret);

        self.refresh_all();

        self.term.hide_cursor().unwrap();
        self.print();
        if res.is_err() {
            self.print_message(&res.err().unwrap().to_string());
        } else {
            match secret_type {
                SecretEdition::Insert => self.print_message(&format!(
                    "added new key to the vault {} -> {}",
                    path, secret
                )),
                SecretEdition::Update => {
                    self.print_message(&format!("updated the secret of {} -> {}", path, secret))
                }
            }
        }
        self.buffered_key.clear();
    }

    fn handle_deleting_key(&mut self) {
        self.term.show_cursor().unwrap();
        self.print_message(&format!(
            "Are you sure you want to delete the key '{}'? (only 'yes' will be accepted): ",
            self.current_list[self.selected_item].name
        ));
        let answer = self.term.read_line().unwrap();
        if answer == "yes" {
            let mut path = self.path.join();
            path.push_str(&self.current_list[self.selected_item].name);
            let res = self.client.delete_secret(&path);
            if res.is_err() {
                self.print_message(&res.err().unwrap().to_string());

                self.term.read_key().unwrap();
            }

            self.refresh_all();
        }

        self.mode = Mode::Navigation;

        self.term.hide_cursor().unwrap();
        self.print();
    }

    fn input_loop(&mut self) {
        loop {
            match self.mode {
                Mode::Navigation => self.handle_navigation(),
                Mode::TypingKey => self.handle_typing_key(),
                Mode::TypingSecret(sm) => self.handle_typing_secret(sm),
                Mode::DeletingKey => self.handle_deleting_key(),
            }

            if self.quit_requested {
                self.term.show_cursor().unwrap();

                break;
            }
        }
    }
}

fn main() {
    let args = Args::parse();

    let mut root = args.root_path;
    if !root.ends_with('/') {
        root += "/";
    }

    let host = args
        .host
        .unwrap_or_else(|| std::env::var("VAULT_ADDR").unwrap());
    let token = args
        .token
        .unwrap_or_else(|| read_to_string(home_dir().unwrap().join(".vault-token")).unwrap());

    let mut vaultwalker = Vaultwalker::new(host, token, root, true);

    ctrlc::set_handler(move || {
        Term::stdout().show_cursor().unwrap();
        std::process::exit(0);
    })
    .expect("Error setting Ctrl-C handler");

    vaultwalker.setup();
    vaultwalker.input_loop();
}
