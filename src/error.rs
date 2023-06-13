use quick_error::quick_error;

/// `Result` type-alias
pub type Result<T> = ::std::result::Result<T, Error>;

quick_error! {
    /// Error enum for vault-rs
    #[derive(Debug)]
    pub enum Error {
        /// `reqwest::Error` errors
        Reqwest(err: ::reqwest::Error) {
            from()
            display("reqwest error: {}", err)
            source(err)
        }
        /// `serde_json::Error`
        SerdeJson(err: ::serde_json::Error) {
            from()
            display("serde_json Error: {}", err)
            source(err)
        }
        /// Vault errors
        Vault(err: String) {
            display("vault error: {}", err)
        }
        /// IO errors
        Io(err: ::std::io::Error) {
            from()
            display("io error: {}", err)
            source(err)
        }
        /// `Url` parsing error
        Url(err: ::url::ParseError) {
          from()
          display("url parse error: {}", err)
          source(err)
      }
    }
}
