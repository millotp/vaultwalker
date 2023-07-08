use std::collections::HashMap;
use std::str::FromStr;

use reqwest::blocking::Client;
use reqwest::{IntoUrl, Method, Url};
use serde::de::DeserializeOwned;
use serde_derive::Deserialize;

use crate::error::{Error, Result};

/// Vault response. Different vault responses have different `data` types, so `D` is used to
/// represent this.
#[derive(Deserialize, Debug)]
pub struct VaultResponse<D> {
    /// Request id
    pub request_id: String,
    /// Lease id
    pub lease_id: Option<String>,
    /// True if renewable
    pub renewable: Option<bool>,
    /// Data
    pub data: Option<D>,
    /// Warnings
    pub warnings: Option<Vec<String>>,
}

/// Response sent by vault when issuing a `LIST` request.
#[derive(Deserialize, Debug)]
pub struct ListResponse {
    /// keys will include the items listed
    pub keys: Vec<String>,
}

pub struct VaultClient {
    client: Client,
    vault_addr: Url,
    token: String,
    cache: HashMap<String, String>,
}

#[derive(Deserialize, Debug)]
pub struct VaultSecret {
    pub secret: Option<String>,
}

impl VaultClient {
    pub fn new<U: IntoUrl, T: Into<String>>(addr: U, token: T) -> Result<VaultClient> {
        let client = Client::new();
        Ok(VaultClient {
            client,
            vault_addr: addr.into_url()?,
            token: token.into(),
            cache: HashMap::new(),
        })
    }

    fn read<T: DeserializeOwned>(
        &mut self,
        method: Method,
        path: &str,
    ) -> Result<VaultResponse<T>> {
        let cache_key = method.to_string() + path;
        if let Some(cache) = self.cache.get(&cache_key) {
            return Ok(serde_json::from_str(cache)?);
        }

        let res = self
            .client
            .request(method, self.vault_addr.join(path)?)
            .header("X-Vault-Token", self.token.clone())
            .header("Content-Type", "application/json")
            .send()?;

        if res.status().is_success() {
            let body = res.text().unwrap();
            // cache the response
            self.cache.insert(cache_key, body.clone());

            Ok(serde_json::from_str(&body)?)
        } else {
            let error_msg = res
                .text()
                .unwrap_or("Could not read vault response.".to_string());
            Err(Error::Vault(format!(
                "Vault request failed `{}`",
                error_msg
            )))
        }
    }

    pub fn get_secret<T: DeserializeOwned + std::fmt::Debug>(&mut self, path: &str) -> Result<T> {
        let res = self.read::<T>(Method::GET, &format!("v1/{}", path))?;
        match res.data {
            Some(data) => Ok(data),
            None => Err(Error::Vault(format!(
                "Vault response did not contain data: {:?}",
                res
            ))),
        }
    }

    pub fn list_secrets(&mut self, path: &str) -> Result<ListResponse> {
        let res = self.read(Method::from_str("LIST").unwrap(), &format!("v1/{}", path))?;
        match res.data {
            Some(data) => Ok(data),
            None => Err(Error::Vault(format!(
                "Vault response did not contain data: {:?}",
                res
            ))),
        }
    }

    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}
