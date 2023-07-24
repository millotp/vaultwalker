# Vaultwalker

![version](https://img.shields.io/crates/v/vaultwalker) ![downloads](https://img.shields.io/crates/d/vaultwalker)

A command line interface to browse and edit [Vault](https://www.vaultproject.io/) secrets.

## How to install

`cargo install vaultwalker`

If you have the vault cli already installed, you can simply use:
```sh
vaultwalker secret/my_company
```

By default it will fetch the vault server address in `$VAULT_ADDR` and the token in the file `~/.vault-token`.

If you want to provide your own login you can use:
```sh
vaultwalker --host <my_vault_server> --token <the vault token> secret/my_company
```

To see all available options use:
```sh
vaultwalker -h
```

## Features

Navigate with the arrow to select any credentials, then use `P` to copy the path to the secret, or `S` to copy the secret itself.

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

## Development

Run with `cargo run secret/my_company`.

### Publishing

`cargo publish`
