mod client;
mod error;

use std::fs::read_to_string;

use client::VaultSecret;
use console::{style, Key, Term};
use home::home_dir;

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

struct Vaultwalker {
    client: VaultClient,
    term: Term,
    path: VaultPath,
    current_list: Vec<VaultEntry>,
    selected_item: usize,
    selected_secret: Option<VaultSecret>,
}

impl Vaultwalker {
    fn new(host: String, token: String) -> Self {
        Self {
            client: VaultClient::new(host, token).unwrap(),
            term: Term::stdout(),
            path: VaultPath::decode("secret/algolia/erc/"),
            current_list: vec![],
            selected_item: 0,
            selected_secret: None,
        }
    }

    fn setup(&mut self) {
        self.term.hide_cursor().unwrap();
        self.term.clear_screen().unwrap();
        self.update_list();
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

    fn input_loop(&mut self) {
        loop {
            match self.term.read_key().unwrap() {
                Key::ArrowDown => {
                    if self.selected_item < self.current_list.len() - 1 {
                        self.selected_item += 1;
                    }
                    self.update_selected_secret();
                    self.term.clear_last_lines(self.current_list.len()).unwrap();
                    self.print();
                }
                Key::ArrowUp => {
                    if self.selected_item > 0 {
                        self.selected_item -= 1;
                    }
                    self.update_selected_secret();
                    self.term.clear_last_lines(self.current_list.len()).unwrap();
                    self.print();
                }
                Key::ArrowRight => {
                    if self.path.entries.len() > 32 {
                        continue;
                    }
                    let entry = self.current_list[self.selected_item].clone();
                    if !entry.is_dir {
                        continue;
                    }
                    self.path.entries.push(entry);
                    let len_before = self.current_list.len();
                    self.update_list();
                    self.selected_item = self.selected_item.min(self.current_list.len() - 1);
                    self.update_selected_secret();
                    self.term.clear_last_lines(len_before).unwrap();
                    self.print();
                }
                Key::ArrowLeft => {
                    if self.path.entries.len() < 4 {
                        continue;
                    }
                    let len_before = self.current_list.len();
                    let last = self.path.entries.pop().unwrap();
                    self.update_list();
                    self.selected_item = self
                        .current_list
                        .iter()
                        .position(|x| x.name == last.name)
                        .unwrap();
                    self.update_selected_secret();
                    self.term.clear_last_lines(len_before).unwrap();
                    self.print();
                }
                Key::Escape => {
                    self.term.show_cursor().unwrap();
                    break;
                }
                _ => (),
            }
        }
    }
}

fn main() {
    let host = std::env::var("VAULT_ADDR").unwrap();
    let token = read_to_string(home_dir().unwrap().join(".vault-token")).unwrap();

    let mut vaultwalker = Vaultwalker::new(host, token);

    ctrlc::set_handler(move || {
        Term::stdout().show_cursor().unwrap();
        std::process::exit(0);
    })
    .expect("Error setting Ctrl-C handler");

    vaultwalker.setup();
    vaultwalker.print();
    vaultwalker.input_loop();
}
