# Vaultwalker

![version](https://img.shields.io/crates/v/vaultwalker) ![downloads](https://img.shields.io/crates/d/vaultwalker)

A command line interface to browse and edit [Vault](https://www.vaultproject.io/) secrets.

## How to install

Install the correct binary for your platform
```sh
curl -s 'https://i.jpillora.com/millotp/vaultwalker!?as=vw' | bash
```

Or build from source:
```sh
cargo install vaultwalker
```

If you have the vault cli already installed, you can simply use:
```sh
vw secret/my_company
```

By default it will fetch the vault server address in `$VAULT_ADDR` and the token in the file `~/.vault-token`.

If you want to provide your own login you can use:
```sh
vw --host <my_vault_server> --token <the vault token> secret/my_company
```

To see all available options use:
```sh
vw -h
```

## Features

Navigate with the arrow to select any credentials (or HJKL), then use `P` to copy the path to the secret, or `S` to copy the secret itself.

To add a new key:
- Navigate to the correct path and press `A`
- Write the name of your key, press `Enter`
- Write the value of the secret, press `Enter` again

To edit a key:
- Select the key you want to edit and press `U`
- Write the new value of the secret, press `Enter`

To delete a key:
- Select the key you want to delete and press `D`
- Enter `yes` to confirm, then `Enter`

To rename a key:
- Select the key you want to rename and press `R`
- Write the new name of the key, press `Enter`

To quit the program press `Q` or `Ctrl+C`.
You can also press `C` to clear the cache refresh the current path.
To view the list of options at any time, press `O`.

## Development

Clone the repository and run `cargo run secret/my_company`.

### Publishing

The changelog is generated with [git-cliff](https://git-cliff.org/), to update it run `git-cliff` and commit the changes.
Before publishing, follow these steps:
- Update the version in `Cargo.toml`
- Push your final commit `git push`
- Check that the CI is passing;
- Create a new tag with the version number `git tag 0.1.0`
- Push the tag `git push --tags`
