use std::{
    collections::{BTreeMap, HashMap},
    time::Duration,
};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_derive::{Deserialize, Serialize};
use ureq::{Agent, AgentBuilder};

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

#[derive(Serialize, Deserialize, Debug)]
pub struct VaultSecret {
    secret: Option<String>,
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
}

impl From<&VaultSecret> for String {
    fn from(val: &VaultSecret) -> Self {
        match &val.secret {
            Some(secret) => secret.to_string(),
            None => serde_json::to_string(&val.other).unwrap(),
        }
    }
}

/// Response sent by vault when issuing a `LIST` request.
#[derive(Deserialize, Debug)]
pub struct ListResponse {
    /// keys will include the items listed
    pub keys: Vec<String>,
}

#[derive(PartialEq, Clone, Copy)]
pub enum FromCache {
    Yes,
    No,
}

pub trait HttpClient {
    fn read<T: DeserializeOwned>(
        &mut self,
        method: &str,
        path: &str,
        cache: FromCache,
    ) -> Result<VaultResponse<T>>;
    fn write<TBody: Serialize>(
        &mut self,
        method: &str,
        path: &str,
        body: Option<TBody>,
    ) -> Result<()>;
    fn clear_cache(&mut self);
}

pub struct UreqClient {
    client: Agent,
    vault_addr: String,
    token: String,
    cache: HashMap<String, String>,
}

impl UreqClient {
    pub fn new(addr: &str, token: &str) -> Self {
        let client = AgentBuilder::new()
            .timeout_read(Duration::from_secs(5))
            .timeout_write(Duration::from_secs(5))
            .build();
        Self {
            client,
            vault_addr: addr.to_string(),
            token: token.into(),
            cache: HashMap::new(),
        }
    }
}

impl HttpClient for UreqClient {
    fn read<T: DeserializeOwned>(
        &mut self,
        method: &str,
        path: &str,
        cache: FromCache,
    ) -> Result<VaultResponse<T>> {
        let cache_key = method.to_string() + path;
        if cache == FromCache::Yes {
            if let Some(cache) = self.cache.get(&cache_key) {
                return Ok(serde_json::from_str(cache)?);
            }
        }

        match self
            .client
            .request(method, &format!("{}/{}", self.vault_addr, path))
            .set("X-Vault-Token", &self.token)
            .set("Content-Type", "application/json")
            .call()
        {
            Ok(res) => {
                let res = res.into_string()?;
                self.cache.insert(cache_key, res.clone());

                Ok(serde_json::from_str(&res)?)
            }
            Err(err) => Err(Error::Ureq(Box::new(err))),
        }
    }

    fn write<TBody: Serialize>(
        &mut self,
        method: &str,
        path: &str,
        body: Option<TBody>,
    ) -> Result<()> {
        let query = self
            .client
            .request(method, &format!("{}/{}", self.vault_addr, path))
            .set("X-Vault-Token", &self.token)
            .set("Content-Type", "application/json");

        let res = match body {
            Some(body) => query.send_string(&serde_json::to_string(&body)?),
            None => query.call(),
        };

        match res {
            Ok(_) => Ok(()),
            Err(err) => Err(Error::Ureq(Box::new(err))),
        }
    }

    fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

pub struct MockClient {}

impl HttpClient for MockClient {
    fn read<T: DeserializeOwned>(
        &mut self,
        method: &str,
        path: &str,
        _cache: FromCache,
    ) -> Result<VaultResponse<T>> {
        Ok(VaultResponse {
            request_id: "mock".to_string(),
            lease_id: None,
            renewable: None,
            data: match method {
                "GET" => Some(serde_json::from_str("{\"secret\":\"value\"}")?),
                "LIST" => Some(match path {
                    "v1/mock/key1/" => serde_json::from_str("{\"keys\":[\"key\"]}")?,
                    _ => serde_json::from_str("{\"keys\":[\"key1/\",\"key2\",\"key3\",\"key4\",\"key5\",\"key6\",\"key7\",\"key8\",\"key9/\",\"key10\",\"key11\",\"key12\",\"key13\",\"key14\",\"key15/\"]}")?,
                }),
                _ => None,
            },
            warnings: None,
        })
    }

    fn write<TBody: Serialize>(
        &mut self,
        _method: &str,
        _path: &str,
        _body: Option<TBody>,
    ) -> Result<()> {
        Ok(())
    }

    fn clear_cache(&mut self) {}
}

pub struct VaultClient<H: HttpClient> {
    client: H,
}

impl<H: HttpClient> VaultClient<H> {
    pub fn new(client: H) -> Self {
        Self { client }
    }

    pub fn get_secret<T: DeserializeOwned + std::fmt::Debug>(
        &mut self,
        path: &str,
        cache: FromCache,
    ) -> Result<T> {
        let res = self
            .client
            .read::<T>("GET", &format!("v1/{}", path), cache)?;
        match res.data {
            Some(data) => Ok(data),
            None => Err(Error::Vault(format!(
                "Vault response did not contain data: {:?}",
                res
            ))),
        }
    }

    pub fn list_secrets(&mut self, path: &str, cache: FromCache) -> Result<ListResponse> {
        let res = self.client.read("LIST", &format!("v1/{}", path), cache)?;
        match res.data {
            Some(data) => Ok(data),
            None => Err(Error::Vault(format!(
                "Vault response did not contain data: {:?}",
                res
            ))),
        }
    }

    pub fn write_secret(&mut self, path: &str, secret: &str) -> Result<()> {
        self.client.write(
            "POST",
            &format!("v1/{}", path),
            Some(VaultSecret {
                secret: Some(secret.to_string()),
                other: BTreeMap::new(),
            }),
        )
    }

    pub fn delete_secret(&mut self, path: &str) -> Result<()> {
        self.client
            .write::<()>("DELETE", &format!("v1/{}", path), None)
    }

    pub fn clear_cache(&mut self) {
        self.client.clear_cache();
    }
}
