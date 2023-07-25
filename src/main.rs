mod client;
mod error;

use std::{
    fs::read_to_string,
    io::{stdout, Write},
};

extern crate clipboard;

use clipboard::{ClipboardContext, ClipboardProvider};

use client::{FromCache, VaultSecret};
use console::{style, Key, Term};
use error::{Error, Result};
use gumdrop::Options;
use home::home_dir;
use termion::{
    clear::{All, CurrentLine},
    cursor::{Goto, Hide, HideCursor, Show},
    screen::{IntoAlternateScreen, ToMainScreen},
    terminal_size,
};

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

fn shorten_string(s: String, max_len: usize) -> String {
    if max_len < s.len() {
        format!("{}...", &s[0..max_len])
    } else {
        s
    }
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
    screen: Box<dyn Write>,
    term: Term,
    clipboard: ClipboardContext,
    mode: Mode,
    quit_requested: bool,
    path: VaultPath,
    root_len: usize,
    current_list: Vec<VaultEntry>,
    selected_item: usize,
    scroll: usize,
    selected_secret: Option<VaultSecret>,
    displayed_message: Option<String>,
    buffered_key: String,
}

impl Vaultwalker {
    fn new(host: String, token: String, root: String, use_alternate_screen: bool) -> Result<Self> {
        let path = VaultPath::decode(&root);
        let vw = Self {
            client: VaultClient::new(&host, &token),
            screen: if use_alternate_screen {
                Box::new(HideCursor::from(stdout().into_alternate_screen()?))
            } else {
                Box::new(HideCursor::from(stdout()))
            },
            term: Term::stdout(),
            clipboard: ClipboardProvider::new().unwrap(),
            mode: Mode::Navigation,
            quit_requested: false,
            root_len: path.entries.len(),
            path,
            current_list: vec![],
            selected_item: 0,
            scroll: 0,
            selected_secret: None,
            displayed_message: None,
            buffered_key: String::new(),
        };

        Ok(vw)
    }

    fn setup(&mut self) -> Result<()> {
        write!(self.screen, "{}", Hide)?;
        self.update_list(FromCache::No)?;
        self.print()?;
        self.print_controls()?;
        self.screen.flush()?;

        Ok(())
    }

    fn update_list(&mut self, cache: FromCache) -> Result<()> {
        let path = self.path.join();
        let res = self.client.list_secrets(&path, cache)?;
        self.current_list = res.keys.iter().map(|x| VaultEntry::decode(x)).collect();

        Ok(())
    }

    fn update_selected_secret(&mut self, cache: FromCache) -> Result<()> {
        if self.current_list[self.selected_item].is_dir {
            self.selected_secret = None;

            return Ok(());
        }
        let mut path = self.path.join();
        path.push_str(&self.current_list[self.selected_item].name);
        let res = self.client.get_secret(&path, cache)?;
        self.selected_secret = Some(res);

        Ok(())
    }

    fn refresh_all(&mut self) -> Result<()> {
        self.update_list(FromCache::No)?;
        self.update_selected_secret(FromCache::No)?;

        Ok(())
    }

    fn selected_line_for_current_mode(
        &self,
        item: &VaultEntry,
        max_width: usize,
    ) -> Result<String> {
        match self.mode {
            Mode::Navigation | Mode::DeletingKey => {
                let mut line = format!("> {}{}", item.name, if item.is_dir { "/" } else { "" });

                let remaining = if max_width < line.len() + 7 {
                    0
                } else {
                    max_width - line.len() - 7
                };

                if let Some(secret) = self.selected_secret.as_ref() {
                    if let Some(secret) = secret.secret.as_ref() {
                        line.push_str(&format!(
                            " -> {}",
                            &style(shorten_string(secret.clone(), remaining))
                                .bold()
                                .bright()
                        ));
                    }
                }

                Ok(line)
            }
            Mode::TypingKey => Ok(format!(
                "> {}{}",
                item.name,
                if item.is_dir { "/" } else { "" }
            )),
            Mode::TypingSecret(_) => Ok(format!(
                "> {}{} -> ",
                item.name,
                if item.is_dir { "/" } else { "" }
            )),
        }
    }

    fn print(&mut self) -> Result<()> {
        write!(self.screen, "{}{}", All, Goto(1, 1))?;
        let (width, height) = terminal_size()?;

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

        if self.selected_item <= self.scroll {
            self.scroll = self.selected_item - if self.selected_item == 0 { 0 } else { 1 };
        }

        if self.selected_item - self.scroll >= height as usize - 3 {
            self.scroll = self.selected_item + 3
                - height as usize
                - if self.selected_item == self.current_list.len() + extended_item.len() - 1 {
                    1
                } else {
                    0
                };
        }

        let mut len_selected = 0;
        let prefix_len = self.path.len() + 1;
        for (i, item) in self
            .current_list
            .iter()
            .chain(extended_item.iter())
            .enumerate()
            .skip(self.scroll)
            .take(height as usize - 1)
        {
            let mut line = String::new();
            if i == self.scroll {
                line.push_str(&format!("{} ", &style(self.path.join()).bold().bright()));
            } else {
                line.push_str(&format!("{:prefix$}", "", prefix = prefix_len));
            }

            if i == self.selected_item {
                line.push_str(&self.selected_line_for_current_mode(
                    item,
                    (width as i32 - line.len() as i32).max(3) as usize,
                )?);
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

            self.screen.write_all(line.as_bytes())?;
        }

        match self.mode {
            Mode::TypingKey | Mode::TypingSecret(_) => {
                write!(
                    self.screen,
                    "{}",
                    Goto(
                        len_selected as u16 + 1,
                        self.selected_item as u16 + 1 - self.scroll as u16
                    )
                )?;
            }
            _ => (),
        };

        self.screen.flush()?;

        Ok(())
    }

    fn print_message(&mut self, message: &str) -> Result<()> {
        if self
            .displayed_message
            .as_ref()
            .is_some_and(|m| m == message)
        {
            return Ok(());
        }

        self.displayed_message = Some(message.to_owned());
        write!(
            self.screen,
            "{}{}{}",
            Goto(1, 10000),
            CurrentLine,
            style(message).black().on_white()
        )?;
        self.screen.flush()?;

        Ok(())
    }

    fn print_controls(&mut self) -> Result<()> {
        self.print_message(
            "navigate with arrows      P: copy path      S: copy secret      A: create secret      U: update secret      D: delete secret      Q: quit      C: clear cache",
        )
    }

    fn handle_navigation(&mut self) -> Result<()> {
        let mut needs_refresh = false;
        match self.term.read_key()? {
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
                    return Ok(());
                }
                let entry = self.current_list[self.selected_item].clone();
                if !entry.is_dir {
                    return Ok(());
                }
                self.path.entries.push(entry);
                self.update_list(FromCache::Yes)?;
                self.selected_item = self.selected_item.min(self.current_list.len() - 1);
                self.scroll = 0;
                needs_refresh = true;
            }
            Key::ArrowLeft | Key::Char('h') => {
                if self.path.entries.len() < self.root_len + 1 {
                    return Ok(());
                }
                let last = self.path.entries.pop().unwrap();
                self.update_list(FromCache::Yes)?;
                self.selected_item = self
                    .current_list
                    .iter()
                    .position(|x| x.name == last.name)
                    .unwrap();
                self.scroll = 0;
                needs_refresh = true;
            }
            Key::Char('c') => {
                self.client.clear_cache();
                self.update_list(FromCache::Yes)?;
                self.update_selected_secret(FromCache::Yes)?;

                needs_refresh = true;
            }
            Key::Char('p') => {
                let mut path = self.path.join();
                path.push_str(&self.current_list[self.selected_item].name);
                self.clipboard.set_contents(path).unwrap();

                self.print_message("path copied to clipboard")?;
            }
            Key::Char('s') => {
                let entry = self.current_list[self.selected_item].clone();
                if entry.is_dir {
                    return Ok(());
                }

                if let Some(secret) = self.selected_secret.as_ref() {
                    if let Some(secret) = secret.secret.as_ref() {
                        self.clipboard.set_contents(secret.clone()).unwrap();

                        self.print_message("secret copied to clipboard")?;
                    }
                }
            }
            Key::Char('a') => {
                self.selected_item = self.current_list.len();
                write!(self.screen, "{}", Show)?;
                self.mode = Mode::TypingKey;

                needs_refresh = true;
            }
            Key::Char('u') => {
                let entry = self.current_list[self.selected_item].clone();
                if entry.is_dir {
                    return Ok(());
                }

                write!(self.screen, "{}", Show)?;
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
                self.update_selected_secret(FromCache::Yes)?;
            }

            self.print()?;
            self.displayed_message = None;
        }

        Ok(())
    }

    fn handle_typing_key(&mut self) -> Result<()> {
        self.buffered_key = self.term.read_line()?;
        self.mode = Mode::TypingSecret(SecretEdition::Insert);

        self.print()
    }

    fn handle_typing_secret(&mut self, secret_type: SecretEdition) -> Result<()> {
        let secret = self.term.read_line()?;
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

        self.refresh_all()?;

        write!(self.screen, "{}", Hide)?;
        self.print()?;
        if let Err(err) = res {
            self.print_message(&err.to_string())?;
        } else {
            match secret_type {
                SecretEdition::Insert => self.print_message(&format!(
                    "added new key to the vault {} -> {}",
                    path, secret
                ))?,
                SecretEdition::Update => {
                    self.print_message(&format!("updated the secret of {} -> {}", path, secret))?
                }
            }
        }
        self.buffered_key.clear();

        Ok(())
    }

    fn handle_deleting_key(&mut self) -> Result<()> {
        write!(self.screen, "{}", Show)?;
        self.print_message(&format!(
            "Are you sure you want to delete the key '{}'? (only 'yes' will be accepted): ",
            self.current_list[self.selected_item].name
        ))?;
        let answer = self.term.read_line()?;
        if answer == "yes" {
            let mut path = self.path.join();
            path.push_str(&self.current_list[self.selected_item].name);
            let res = self.client.delete_secret(&path);
            if let Err(err) = res {
                self.print_message(&err.to_string())?;

                self.term.read_key()?;
            }

            self.refresh_all()?;
        }

        self.mode = Mode::Navigation;

        write!(self.screen, "{}", Hide)?;

        self.print()
    }

    fn input_loop(&mut self) -> Result<()> {
        loop {
            match self.mode {
                Mode::Navigation => self.handle_navigation(),
                Mode::TypingKey => self.handle_typing_key(),
                Mode::TypingSecret(sm) => self.handle_typing_secret(sm),
                Mode::DeletingKey => self.handle_deleting_key(),
            }?;

            if self.quit_requested {
                return Ok(());
            }
        }
    }
}

#[derive(Options)]
struct Args {
    #[options(help_flag)]
    help: bool,

    #[options(free, required, help = "Path to the root of the vault")]
    root_path: String,

    #[options(help = "URL of the vault server, defaults to $VAULT_ADDR", short = "H")]
    host: Option<String>,

    #[options(help = "Vault token, default to the value in ~/.vault-token")]
    token: Option<String>,
}

fn run(host: String, token: String, root: String) -> Result<()> {
    let mut vaultwalker = Vaultwalker::new(host, token, root, true)?;

    vaultwalker.setup()?;
    vaultwalker.input_loop()
}

fn main() {
    let opts = Args::parse_args_default_or_exit();

    let mut root = opts.root_path;
    if !root.ends_with('/') {
        root += "/";
    }

    let host = opts
        .host
        .unwrap_or_else(|| std::env::var("VAULT_ADDR").unwrap());
    let token = opts
        .token
        .unwrap_or_else(|| read_to_string(home_dir().unwrap().join(".vault-token")).unwrap());

    ctrlc::set_handler(|| {
        write!(stdout(), "{}{}", ToMainScreen, Show).unwrap();
        std::process::exit(0);
    })
    .expect("Error setting Ctrl-C handler");

    run(host, token, root).unwrap_or_else(|err: Error| {
        println!("{}", err);
        std::process::exit(0);
    });
}
