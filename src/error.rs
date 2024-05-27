use quick_error::quick_error;

/// `Result` type-alias
pub type Result<T> = ::std::result::Result<T, Error>;

quick_error! {
    /// Error enum for vault-rs
    #[derive(Debug)]
    pub enum Error {
        Ureq(err: Box<ureq::Error>) {
            from()
            display("ureq error: {}", err)
            source(err)
        }
        /// `serde_json::Error`
        SerdeJson(err: serde_json::Error) {
            from()
            display("serde_json Error: {}", err)
            source(err)
        }
        /// Vault errors
        Vault(err: String) {
            display("vault error: {}", err)
        }
        /// Application errors
        Application(err: String) {
            display("{}", err)
        }
        /// IO errors
        Io(err: std::io::Error) {
            from()
            display("io error: {}", err)
            source(err)
        }
    }
}
