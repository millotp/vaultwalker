mod client;
mod error;

use std::{
    fmt,
    fs::read_to_string,
    io::{stdin, stdout},
};

extern crate clipboard;

use clipboard::{ClipboardContext, ClipboardProvider};
use crossterm::{
    cursor::{self, MoveDown, MoveTo, MoveToNextLine},
    event::{read, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    style::{Print, StyledContent, Stylize},
    terminal::{
        self, disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};

use client::{FromCache, HttpClient, MockClient, UreqClient, VaultSecret};
use error::{Error, Result};
use gumdrop::Options;
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

impl fmt::Display for VaultEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}{}", self.name, if self.is_dir { "/" } else { "" })
    }
}

struct VaultPath {
    entries: Vec<VaultEntry>,
}

impl VaultPath {
    fn join(&self) -> String {
        self.entries
            .iter()
            .fold(String::new(), |acc, item| format!("{}{}", acc, item))
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

fn shorten_string(s: impl Into<String>, max_len: usize) -> String {
    let s = s.into();
    if max_len < s.len() {
        format!("{}...", &s[0..max_len])
    } else {
        s
    }
}

fn read_line() -> Result<String> {
    execute!(stdout(), cursor::Show)?;
    disable_raw_mode()?;
    let mut line = String::new();
    stdin().read_line(&mut line)?;
    enable_raw_mode()?;
    execute!(stdout(), cursor::Hide)?;

    while line.ends_with('\n') {
        line.pop();
    }

    Ok(line)
}

#[derive(PartialEq, Copy, Clone)]
enum EditMode {
    Insert,
    Update,
}

#[derive(PartialEq)]
enum Mode {
    Navigation,
    TypingKey(EditMode),
    TypingSecret(EditMode),
    DeletingKey,
}

struct Vaultwalker<H: HttpClient> {
    client: VaultClient<H>,
    clipboard: Option<ClipboardContext>,
    mode: Mode,
    quit_requested: bool,
    path: VaultPath,
    root_len: usize,
    current_list: Vec<VaultEntry>,
    selected_item: usize,
    previous_selected_item: usize,
    scroll: usize,
    selected_secret: Option<VaultSecret>,
    displayed_message: Option<String>,
    buffered_key: String,
}

impl<H: HttpClient> Vaultwalker<H> {
    fn new(http_client: H, root: String) -> Result<Self> {
        let path = VaultPath::decode(&root);
        let vw = Self {
            client: VaultClient::new(http_client),
            clipboard: ClipboardProvider::new().ok(),
            mode: Mode::Navigation,
            quit_requested: false,
            root_len: path.entries.len(),
            path,
            current_list: vec![],
            selected_item: 0,
            previous_selected_item: 0,
            scroll: 0,
            selected_secret: None,
            displayed_message: None,
            buffered_key: String::new(),
        };

        Ok(vw)
    }

    fn setup(&mut self) -> Result<()> {
        execute!(stdout(), cursor::Hide, EnterAlternateScreen)?;
        enable_raw_mode()?;
        self.update_list(FromCache::No)?;
        self.update_selected_secret(FromCache::No)?;
        self.print()?;
        self.print_controls()?;

        Ok(())
    }

    fn get_selected_path(&self) -> String {
        self.path.join() + &self.current_list[self.selected_item].name
    }

    fn rename_key(&mut self, new_key: &str) -> Result<()> {
        // check if the key already exists
        self.update_list(FromCache::No)?;
        if self.current_list.iter().any(|x| x.name == new_key) {
            return Err(Error::Application(format!(
                "the key '{}' already exists",
                new_key
            )));
        }

        // read the secret from the old key
        self.update_selected_secret(FromCache::No)?;
        let secret = self.selected_secret.as_ref().unwrap();

        // write the secret to the new key
        let new_path = format!("{}{}", self.path.join(), new_key);
        self.client
            .write_secret(&new_path, &<&VaultSecret as Into<String>>::into(secret))?;

        // delete the old key
        self.client.delete_secret(&self.get_selected_path())?;
        self.set_selected_item(new_key, FromCache::No)?;
        self.print()?;
        self.print_info("successfully renamed the key")
    }

    fn update_list(&mut self, cache: FromCache) -> Result<()> {
        let path = self.path.join();
        let res = self.client.list_secrets(&path, cache)?;
        self.current_list = res.keys.iter().map(|x| VaultEntry::decode(x)).collect();

        Ok(())
    }

    fn update_selected_secret(&mut self, cache: FromCache) -> Result<()> {
        // this is a security to avoid panic
        if self.selected_item >= self.current_list.len() {
            self.selected_secret = None;

            return Ok(());
        }

        if self.current_list[self.selected_item].is_dir {
            self.selected_secret = None;

            return Ok(());
        }

        let res = self.client.get_secret(&self.get_selected_path(), cache)?;
        self.selected_secret = Some(res);

        Ok(())
    }

    fn set_selected_item(&mut self, key: &str, cache: FromCache) -> Result<()> {
        self.update_list(cache)?;
        self.selected_item = self
            .current_list
            .iter()
            .position(|x| x.name == key)
            .unwrap_or(self.previous_selected_item);
        self.update_selected_secret(cache)
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
                let mut line = format!("> {}", item);

                let remaining = if max_width < line.len() + 7 {
                    0
                } else {
                    max_width - line.len() - 7
                };

                if let Some(secret) = self.selected_secret.as_ref() {
                    line.push_str(&format!(" -> {}", shorten_string(secret, remaining).bold()));
                }

                Ok(line)
            }
            Mode::TypingKey(EditMode::Insert) => Ok(format!("> {}", item)),
            Mode::TypingKey(EditMode::Update) => Ok("> ".to_string()),
            Mode::TypingSecret(_) => Ok(format!("> {} -> ", item)),
        }
    }

    fn print(&mut self) -> Result<()> {
        // on windows, the cursor must be hidden again when the terminal is cleared
        execute!(stdout(), Clear(ClearType::All), cursor::Hide, MoveTo(0, 0))?;
        let (width, height) = terminal::size()?;

        let mut extended_item = Vec::new();
        match self.mode {
            Mode::TypingKey(EditMode::Insert) => extended_item.push(VaultEntry {
                name: self.buffered_key.clone(),
                is_dir: false,
            }),
            Mode::TypingSecret(_) => extended_item.push(VaultEntry {
                name: self.buffered_key.clone(),
                is_dir: false,
            }),
            _ => (),
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
            let mut line = if i == self.scroll {
                format!("{} ", self.path.join().bold())
            } else {
                format!("{:prefix$}", "", prefix = prefix_len)
            };

            if i == self.selected_item {
                line.push_str(&self.selected_line_for_current_mode(
                    item,
                    (width as i32 - line.len() as i32).max(3) as usize,
                )?);
                len_selected = line.len();
                if i == self.scroll {
                    // if the selected item is the first item, we need to remove the bold prefix
                    len_selected -= 8;
                }
            } else {
                line.push_str(&format!("  {}", item));
            }

            execute!(stdout(), Print(line), MoveToNextLine(1))?;
        }

        match self.mode {
            Mode::TypingKey(_) | Mode::TypingSecret(_) => {
                execute!(
                    stdout(),
                    MoveTo(
                        len_selected as u16,
                        self.selected_item as u16 - self.scroll as u16
                    )
                )?;
            }
            _ => (),
        };

        Ok(())
    }

    fn print_message_raw(&mut self, message: StyledContent<String>) -> Result<()> {
        if self
            .displayed_message
            .as_ref()
            .is_some_and(|m| m == message.content())
        {
            return Ok(());
        }

        self.displayed_message = Some(message.content().clone());
        let (width, height) = terminal::size()?;
        let offset = 1 + message.content().len() / width as usize;
        execute!(
            stdout(),
            MoveTo(0, (height - offset as u16).max(0)),
            Clear(ClearType::CurrentLine),
            Print(message),
        )?;

        Ok(())
    }

    fn print_info(&mut self, message: &str) -> Result<()> {
        self.print_message_raw(format!(" {} ", message).black().on_white())
    }

    fn print_error(&mut self, err: Error) -> Result<()> {
        self.print_message_raw(format!(" {} ", err).white().on_red())
    }

    fn print_controls(&mut self) -> Result<()> {
        self.print_info(
            "Navigate with arrows or HJKL    copy [P]ath    copy [S]ecret    [A]dd secret    [R]ename key    [U]pdate secret    [D]elete secret    [Q]uit    [C]lear cache    [O]pen help",
        )
    }

    fn handle_navigation(&mut self) -> Result<()> {
        let mut needs_refresh = false;
        if let Event::Key(event) = read()? {
            if event.kind != KeyEventKind::Press {
                return Ok(());
            }
            match event.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.selected_item < self.current_list.len() - 1 {
                        self.selected_item += 1;
                    }
                    needs_refresh = true;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.selected_item > 0 {
                        self.selected_item -= 1;
                    }
                    needs_refresh = true;
                }
                KeyCode::Right | KeyCode::Char('l') => {
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
                KeyCode::Left | KeyCode::Char('h') => {
                    if self.path.entries.len() < self.root_len + 1 {
                        return Ok(());
                    }
                    let last = self.path.entries.pop().unwrap();
                    self.set_selected_item(&last.name, FromCache::Yes)?;
                    self.scroll = 0;
                    needs_refresh = true;
                }
                KeyCode::Char('c') => {
                    if event.modifiers.contains(KeyModifiers::CONTROL) {
                        self.quit_requested = true
                    } else {
                        self.client.clear_cache();
                        self.update_list(FromCache::Yes)?;
                        self.update_selected_secret(FromCache::Yes)?;
                    }

                    needs_refresh = true;
                }
                KeyCode::Char('o') => {
                    self.print_controls()?;
                }
                KeyCode::Char('p') if self.clipboard.is_some() => {
                    let path = self.get_selected_path();
                    self.clipboard.as_mut().unwrap().set_contents(path).unwrap();

                    self.print_info("path copied to clipboard")?;
                }
                KeyCode::Char('s') if self.clipboard.is_some() => {
                    let entry = &self.current_list[self.selected_item];
                    if entry.is_dir {
                        return Ok(());
                    }

                    if let Some(secret) = self.selected_secret.as_ref() {
                        let secret = secret.into();
                        self.clipboard
                            .as_mut()
                            .unwrap()
                            .set_contents(secret)
                            .unwrap();

                        self.print_info("secret copied to clipboard")?;
                    }
                }
                KeyCode::Char('a') => {
                    self.previous_selected_item = self.selected_item;
                    self.selected_item = self.current_list.len();
                    self.mode = Mode::TypingKey(EditMode::Insert);

                    needs_refresh = true;
                }
                KeyCode::Char('u') => {
                    let entry = &self.current_list[self.selected_item];
                    if entry.is_dir {
                        return Err(Error::Application(
                            "cannot update a directory, please select a key".to_owned(),
                        ));
                    }

                    self.mode = Mode::TypingSecret(EditMode::Update);

                    needs_refresh = true;
                }
                KeyCode::Char('r') => {
                    let entry = &self.current_list[self.selected_item];
                    if entry.is_dir {
                        return Err(Error::Application(
                            "cannot rename a directory, please select a key".to_owned(),
                        ));
                    }

                    self.mode = Mode::TypingKey(EditMode::Update);

                    needs_refresh = true;
                }
                KeyCode::Char('d') => {
                    let entry = &self.current_list[self.selected_item];
                    if entry.is_dir {
                        return Err(Error::Application(
                            "cannot delete a directory, please select a key".to_owned(),
                        ));
                    }
                    self.mode = Mode::DeletingKey;

                    needs_refresh = true;
                }
                KeyCode::Esc | KeyCode::Char('q') => self.quit_requested = true,
                _ => (),
            }
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

    fn handle_typing_key(&mut self, edit_mode: EditMode) -> Result<()> {
        let key = read_line()?;
        if key.is_empty() {
            self.mode = Mode::Navigation;
            self.buffered_key.clear();
            self.selected_item = self.previous_selected_item;

            return Err(Error::Application("the key must not be empty".to_owned()));
        }

        if key.ends_with('/') {
            self.mode = Mode::Navigation;
            self.buffered_key.clear();
            self.selected_item = self.previous_selected_item;

            return Err(Error::Application(
                "to create a directory, please specify the key name, e.g. directory/keyName"
                    .to_owned(),
            ));
        }

        match edit_mode {
            EditMode::Insert => {
                self.buffered_key = key;
                self.mode = Mode::TypingSecret(EditMode::Insert);
                self.print()
            }
            EditMode::Update => {
                self.mode = Mode::Navigation;
                self.rename_key(&key)
            }
        }
    }

    fn handle_typing_secret(&mut self, secret_type: EditMode) -> Result<()> {
        let secret = read_line()?;
        self.mode = Mode::Navigation;
        let key = match secret_type {
            EditMode::Insert => self.buffered_key.clone(),
            EditMode::Update => self.current_list[self.selected_item].name.clone(),
        };
        let path = format!("{}{}", self.path.join(), key);

        self.client.write_secret(&path, &secret)?;
        self.set_selected_item(&key, FromCache::No)?;
        self.print()?;

        match secret_type {
            EditMode::Insert => self.print_info(&format!(
                "added new key to the vault {} -> {}",
                path, secret
            ))?,
            EditMode::Update => {
                self.print_info(&format!("updated the secret of {} -> {}", path, secret))?
            }
        }

        self.buffered_key.clear();

        Ok(())
    }

    fn handle_deleting_key(&mut self) -> Result<()> {
        self.print_info(&format!(
            "Are you sure you want to delete the key '{}'? (only 'yes' will be accepted): ",
            self.current_list[self.selected_item].name
        ))?;
        execute!(stdout(), Print(" "))?;

        let answer = read_line()?;
        // because the user presses on new line, we need to reset it
        self.print()?;
        self.mode = Mode::Navigation;

        if answer == "yes" {
            let mut path = self.path.join();
            path.push_str(&self.current_list[self.selected_item].name);
            self.client.delete_secret(&path)?;

            // if this is the only item in the list, we need to climb up
            while self.current_list.len() == 1 {
                let last = self.path.entries.pop().unwrap();
                self.set_selected_item(&last.name, FromCache::Yes)?;
                self.scroll = 0;
            }

            // if the last item was deleted, we need to move the selection up
            if self.selected_item >= self.current_list.len() - 1 {
                self.selected_item = self.current_list.len() - 1;
            }

            self.refresh_all()?;
            self.print()?;
            self.print_info(&format!("deleted the key '{}'", path))
        } else {
            self.print()?;
            self.print_error(Error::Application(format!(
                "received '{}', the key was not deleted",
                answer
            )))
        }
    }

    fn input_loop(&mut self) -> Result<()> {
        loop {
            let err = match self.mode {
                Mode::Navigation => self.handle_navigation(),
                Mode::TypingKey(em) => self.handle_typing_key(em),
                Mode::TypingSecret(em) => self.handle_typing_secret(em),
                Mode::DeletingKey => self.handle_deleting_key(),
            };

            if let Err(err) = err {
                self.print()?;
                self.print_error(err)?;
            }

            if self.quit_requested {
                disable_raw_mode()?;
                execute!(stdout(), LeaveAlternateScreen, cursor::Show)?;
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

struct ParsedArgs {
    host: String,
    token: String,
    root: String,
}

fn run(host: String, token: String, root: String) -> Result<()> {
    if root == "mock/" {
        let mock_client = MockClient {};
        let mut vaultwalker = Vaultwalker::new(mock_client, root)?;
        vaultwalker.setup()?;
        vaultwalker.input_loop()
    } else {
        let http_client = UreqClient::new(&host, &token);
        let mut vaultwalker = Vaultwalker::new(http_client, root)?;
        vaultwalker.setup()?;
        vaultwalker.input_loop()
    }
}

fn parse_args(opts: Args) -> Result<ParsedArgs> {
    let mut root = opts.root_path;
    if !root.ends_with('/') {
        root += "/";
    }

    let host = opts.host.or_else(|| std::env::var("VAULT_ADDR").ok()).ok_or(Error::Application(
        "please specify the vault server URL with -H option or set the VAULT_ADDR environment variable".to_owned(),
    ))?;
    let token = opts.token.or_else(|| read_to_string(home_dir().unwrap().join(".vault-token")).ok()).ok_or(Error::Application(
        "cannot find ~/.vault-token file, please specify the token with -t option or use the 'vault login' command to create it".to_owned()
    ))?;

    Ok(ParsedArgs { host, token, root })
}

fn main() {
    let ParsedArgs { host, token, root } = parse_args(Args::parse_args_default_or_exit())
        .unwrap_or_else(|err: Error| {
            eprintln!("{}", err);
            std::process::exit(2);
        });

    ctrlc::set_handler(|| {
        disable_raw_mode().unwrap();
        execute!(stdout(), LeaveAlternateScreen, cursor::Show).unwrap();
        std::process::exit(1);
    })
    .expect("Error setting Ctrl-C handler");

    run(host, token, root).unwrap_or_else(|err: Error| {
        disable_raw_mode().unwrap();
        execute!(
            stdout(),
            LeaveAlternateScreen,
            cursor::Show,
            MoveDown(20000),
            Print(err)
        )
        .unwrap();
        std::process::exit(1);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vault_entry_display() {
        let entry = VaultEntry {
            name: "test".to_owned(),
            is_dir: false,
        };

        assert_eq!(format!("{}", entry), "test");

        let entry = VaultEntry {
            name: "test".to_owned(),
            is_dir: true,
        };

        assert_eq!(format!("{}", entry), "test/");
    }

    #[test]
    fn test_vault_path() {
        let path = VaultPath::decode("test/dir");

        assert_eq!(path.entries.len(), 2);
        assert_eq!(path.entries[0].name, "test");
        assert!(path.entries[0].is_dir);
        assert_eq!(path.entries[1].name, "dir");
        assert!(!path.entries[1].is_dir);

        assert_eq!(path.len(), 9);
        assert_eq!(path.join(), "test/dir");
    }

    #[test]
    fn test_shorten_string() {
        assert_eq!(shorten_string("test", 10), "test");
        assert_eq!(shorten_string("test", 3), "tes...");
    }

    #[test]
    fn test_parse_args() {
        let args = Args {
            help: false,
            root_path: "mock".to_owned(),
            host: Some("http://localhost:8200".to_owned()),
            token: Some("test_token".to_owned()),
        };
        let parsed = parse_args(args).unwrap();

        assert_eq!(parsed.host, "http://localhost:8200");
        assert_eq!(parsed.token, "test_token");
        assert_eq!(parsed.root, "mock/");
    }

    #[test]
    fn test_vaultwalker() {
        let mut vw = Vaultwalker::new(MockClient {}, "mock/".to_owned()).unwrap();

        // test the initial state
        assert!(vw.update_list(FromCache::No).is_ok());
        assert_eq!(vw.selected_item, 0);
        assert_eq!(vw.current_list.len(), 15);
        assert_eq!(vw.current_list[0].name, "key1");
        assert!(vw.current_list[0].is_dir);
        assert_eq!(vw.get_selected_path(), "mock/key1");
        assert!(vw.selected_secret.is_none());
        assert!(vw.update_selected_secret(FromCache::No).is_ok());
        assert!(vw.refresh_all().is_ok());
        assert_eq!(
            vw.selected_line_for_current_mode(&vw.current_list[0], 80)
                .unwrap(),
            "> key1/"
        );

        // test with a key
        assert!(vw.set_selected_item("key2", FromCache::No).is_ok());
        assert_eq!(vw.selected_item, 1);
        assert_eq!(vw.get_selected_path(), "mock/key2");
        assert!(vw.selected_secret.is_some());
        let secret = vw.selected_secret.as_ref().unwrap();
        assert_eq!(<&VaultSecret as Into<String>>::into(secret), "value");
        assert_eq!(
            vw.selected_line_for_current_mode(&vw.current_list[0], 80)
                .unwrap(),
            "> key1/ -> \u{1b}[1mvalue\u{1b}[0m"
        );
        assert_eq!(
            vw.selected_line_for_current_mode(&vw.current_list[0], 15)
                .unwrap(),
            "> key1/ -> \u{1b}[1mv...\u{1b}[0m"
        );

        // test to rename a key into a key that already exists
        assert_eq!(
            vw.rename_key("key3").unwrap_err().to_string(),
            "the key 'key3' already exists"
        );
    }
}
