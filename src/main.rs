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

struct Vaultwalker {
    client: VaultClient,
    screen: AlternateScreen<Stdout>,
    term: Term,
    clipboard: ClipboardContext,
    path: VaultPath,
    root_len: usize,
    current_list: Vec<VaultEntry>,
    selected_item: usize,
    selected_secret: Option<VaultSecret>,
}

impl Vaultwalker {
    fn new(host: String, token: String, root: String) -> Self {
        let path = VaultPath::decode(&root);
        Self {
            client: VaultClient::new(host, token).unwrap(),
            screen: stdout().into_alternate_screen().unwrap(),
            term: Term::stdout(),
            clipboard: ClipboardProvider::new().unwrap(),
            root_len: path.entries.len(),
            path,
            current_list: vec![],
            selected_item: 0,
            selected_secret: None,
        }
    }

    fn setup(&mut self) {
        self.term.hide_cursor().unwrap();
        self.term.clear_screen().unwrap();
        self.update_list();
        self.print();
        self.print_controls();
        self.screen.flush().unwrap();
    }

    fn update_list(&mut self) {
        let path = self.path.join();
        let res = self.client.list_secrets(&path).unwrap();
        self.current_list = res.keys.iter().map(|x| VaultEntry::decode(x)).collect();
    }

    fn update_selected_secret(&mut self) {
        if self.current_list[self.selected_item].is_dir {
            self.selected_secret = None;
            return;
        }
        let mut path = self.path.join();
        path.push_str(&self.current_list[self.selected_item].name);
        let res = self.client.get_secret(&path).unwrap();
        self.selected_secret = Some(res);
    }

    fn print(&self) {
        let prefix_len = self.path.len() + 1;
        for (i, item) in self.current_list.iter().enumerate() {
            let mut line = String::new();
            if i == 0 {
                line.push_str(&format!("{} ", &style(self.path.join()).bold().bright()));
            } else {
                line.push_str(&format!("{:prefix$}", "", prefix = prefix_len));
            }

            line.push_str(&format!(
                "{} {}{}",
                if i == self.selected_item { ">" } else { " " },
                item.name,
                if item.is_dir { "/" } else { "" }
            ));

            if let Some(secret) = self.selected_secret.as_ref() {
                if secret.secret.is_some() && i == self.selected_item {
                    line.push_str(&format!(
                        " -> {}",
                        &style(&secret.secret.as_ref().unwrap()).bold().bright()
                    ));
                }
            }

            self.term.write_line(&line).unwrap();
        }
    }

    fn print_message(&mut self, message: &str) {
        self.term.move_cursor_down(10000).unwrap();
        self.term
            .write_all(style(message).black().on_white().to_string().as_bytes())
            .unwrap();
    }

    fn print_controls(&mut self) {
        self.print_message(
            "navigate with arrows      C: clear cache      P: copy path      S: copy secret",
        );
    }

    fn input_loop(&mut self) {
        loop {
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
                        continue;
                    }
                    let entry = self.current_list[self.selected_item].clone();
                    if !entry.is_dir {
                        continue;
                    }
                    self.path.entries.push(entry);
                    self.update_list();
                    self.selected_item = self.selected_item.min(self.current_list.len() - 1);
                    needs_refresh = true;
                }
                Key::ArrowLeft | Key::Char('h') => {
                    if self.path.entries.len() < self.root_len + 1 {
                        continue;
                    }
                    let last = self.path.entries.pop().unwrap();
                    self.update_list();
                    self.selected_item = self
                        .current_list
                        .iter()
                        .position(|x| x.name == last.name)
                        .unwrap();
                    needs_refresh = true;
                }
                Key::Char('c') => {
                    self.client.clear_cache();
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
                        continue;
                    }

                    if let Some(secret) = self.selected_secret.as_ref() {
                        if let Some(secret) = secret.secret.as_ref() {
                            self.clipboard.set_contents(secret.clone()).unwrap();

                            self.print_message("secret copied to clipboard");
                        }
                    }
                }
                Key::Escape | Key::Char('q') => {
                    self.term.show_cursor().unwrap();
                    break;
                }
                _ => (),
            }

            if needs_refresh {
                self.update_selected_secret();
                self.term.clear_screen().unwrap();
                self.print();
            }
        }
    }
}

fn main() {
    let args = Args::parse();

    let mut root = args.root_path;

    // let mut root = args().nth(1).expect("A root path is required");
    if !root.ends_with('/') {
        root += "/";
    }

    let host = args
        .host
        .unwrap_or_else(|| std::env::var("VAULT_ADDR").unwrap());
    let token = args
        .token
        .unwrap_or_else(|| read_to_string(home_dir().unwrap().join(".vault-token")).unwrap());

    let mut vaultwalker = Vaultwalker::new(host, token, root);

    ctrlc::set_handler(move || {
        Term::stdout().show_cursor().unwrap();
        std::process::exit(0);
    })
    .expect("Error setting Ctrl-C handler");

    vaultwalker.setup();
    vaultwalker.input_loop();
}
