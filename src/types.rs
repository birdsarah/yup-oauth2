use chrono::{DateTime, UTC, TimeZone};
use std::fmt;
use std::str::FromStr;
use hyper;

/// A marker trait for all Flows
pub trait Flow {
    fn type_id() -> FlowType;
}

#[derive(Deserialize)]
pub struct JsonError {
    pub error: String,
    pub error_description: Option<String>,
    pub error_uri: Option<String>,
}

/// Encapsulates all possible results of the `request_token(...)` operation
pub enum RequestError {
    /// Indicates connection failure
    HttpError(hyper::Error),
    /// The OAuth client was not found
    InvalidClient,
    /// Some requested scopes were invalid. String contains the scopes as part of
    /// the server error message
    InvalidScope(String),
    /// A 'catch-all' variant containing the server error and description
    /// First string is the error code, the second may be a more detailed description
    NegativeServerResponse(String, Option<String>),
}

impl From<JsonError> for RequestError {
    fn from(value: JsonError) -> RequestError {
        match &*value.error {
            "invalid_client" => RequestError::InvalidClient,
            "invalid_scope" => {
                RequestError::InvalidScope(value.error_description
                    .unwrap_or("no description provided".to_string()))
            }
            _ => RequestError::NegativeServerResponse(value.error, value.error_description),
        }
    }
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match *self {
            RequestError::HttpError(ref err) => err.fmt(f),
            RequestError::InvalidClient => "Invalid Client".fmt(f),
            RequestError::InvalidScope(ref scope) => writeln!(f, "Invalid Scope: '{}'", scope),
            RequestError::NegativeServerResponse(ref error, ref desc) => {
                try!(error.fmt(f));
                if let &Some(ref desc) = desc {
                    try!(write!(f, ": {}", desc));
                }
                "\n".fmt(f)
            }
        }
    }
}


/// Represents all implemented token types
#[derive(Clone, PartialEq, Debug)]
pub enum TokenType {
    /// Means that whoever bears the access token will be granted access
    Bearer,
}

impl AsRef<str> for TokenType {
    fn as_ref(&self) -> &'static str {
        match *self {
            TokenType::Bearer => "Bearer",
        }
    }
}

impl FromStr for TokenType {
    type Err = ();
    fn from_str(s: &str) -> Result<TokenType, ()> {
        match s {
            "Bearer" => Ok(TokenType::Bearer),
            _ => Err(()),
        }
    }
}


/// A scheme for use in `hyper::header::Authorization`
#[derive(Clone, PartialEq, Debug)]
pub struct Scheme {
    /// The type of our access token
    pub token_type: TokenType,
    /// The token returned by one of the Authorization Flows
    pub access_token: String,
}

impl hyper::header::Scheme for Scheme {
    fn scheme() -> Option<&'static str> {
        None
    }

    fn fmt_scheme(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {}", self.token_type.as_ref(), self.access_token)
    }
}

impl FromStr for Scheme {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Scheme, &'static str> {
        let parts: Vec<&str> = s.split(' ').collect();
        if parts.len() != 2 {
            return Err("Expected two parts: <token_type> <token>");
        }
        match <TokenType as FromStr>::from_str(parts[0]) {
            Ok(t) => {
                Ok(Scheme {
                    token_type: t,
                    access_token: parts[1].to_string(),
                })
            }
            Err(_) => Err("Couldn't parse token type"),
        }
    }
}

/// Represents a token as returned by OAuth2 servers.
///
/// It is produced by all authentication flows.
/// It authenticates certain operations, and must be refreshed once
/// it reached it's expiry date.
///
/// The type is tuned to be suitable for direct de-serialization from server
/// replies, as well as for serialization for later reuse. This is the reason
/// for the two fields dealing with expiry - once in relative in and once in
/// absolute terms.
///
/// Utility methods make common queries easier, see `expired()`.
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct Token {
    /// used when authenticating calls to oauth2 enabled services.
    pub access_token: String,
    /// used to refresh an expired access_token.
    pub refresh_token: String,
    /// The token type as string - usually 'Bearer'.
    pub token_type: String,
    /// access_token will expire after this amount of time.
    /// Prefer using expiry_date()
    pub expires_in: Option<i64>,
    /// timestamp is seconds since epoch indicating when the token will expire in absolute terms.
    /// use expiry_date() to convert to DateTime.
    pub expires_in_timestamp: Option<i64>,
}

impl Token {
    /// Returns true if we are expired.
    ///
    /// # Panics
    /// * if our access_token is unset
    pub fn expired(&self) -> bool {
        if self.access_token.len() == 0 || self.refresh_token.len() == 0 {
            panic!("called expired() on unset token");
        }
        self.expiry_date() <= UTC::now()
    }

    /// Returns a DateTime object representing our expiry date.
    pub fn expiry_date(&self) -> DateTime<UTC> {
        UTC.timestamp(self.expires_in_timestamp
                          .expect("Tokens without an absolute expiry are invalid"),
                      0)
    }

    /// Adjust our stored expiry format to be absolute, using the current time.
    pub fn set_expiry_absolute(&mut self) -> &mut Token {
        if self.expires_in_timestamp.is_some() {
            assert!(self.expires_in.is_none());
            return self;
        }

        self.expires_in_timestamp = Some(UTC::now().timestamp() + self.expires_in.unwrap());
        self.expires_in = None;
        self
    }
}

/// All known authentication types, for suitable constants
#[derive(Clone, Copy)]
pub enum FlowType {
    /// [device authentication](https://developers.google.com/youtube/v3/guides/authentication#devices). Only works
    /// for certain scopes.
    Device,
    /// [installed app flow](https://developers.google.com/identity/protocols/OAuth2InstalledApp). Required
    /// for Drive, Calendar, Gmail...; Requires user to paste a code from the browser.
    InstalledInteractive,
    /// Same as InstalledInteractive, but uses a redirect: The OAuth provider redirects the user's
    /// browser to a web server that is running on localhost. This may not work as well with the
    /// Windows Firewall, but is more comfortable otherwise. The integer describes which port to
    /// bind to (default: 8080)
    InstalledRedirect(u32),
}

impl AsRef<str> for FlowType {
    /// Converts itself into a URL string
    fn as_ref(&self) -> &'static str {
        match *self {
            FlowType::Device => "https://accounts.google.com/o/oauth2/device/code",
            FlowType::InstalledInteractive => "https://accounts.google.com/o/oauth2/v2/auth",
            FlowType::InstalledRedirect(_) => "https://accounts.google.com/o/oauth2/v2/auth",
        }
    }
}

/// Represents either 'installed' or 'web' applications in a json secrets file.
/// See `ConsoleApplicationSecret` for more information
#[derive(Deserialize, Serialize, Clone, Default)]
pub struct ApplicationSecret {
    /// The client ID.
    pub client_id: String,
    /// The client secret.
    pub client_secret: String,
    /// The token server endpoint URI.
    pub token_uri: String,
    /// The authorization server endpoint URI.
    pub auth_uri: String,
    pub redirect_uris: Vec<String>,

    /// Name of the google project the credentials are associated with
    pub project_id: Option<String>,
    /// The service account email associated with the client.
    pub client_email: Option<String>,
    /// The URL of the public x509 certificate, used to verify the signature on JWTs, such
    /// as ID tokens, signed by the authentication provider.
    pub auth_provider_x509_cert_url: Option<String>,
    ///  The URL of the public x509 certificate, used to verify JWTs signed by the client.
    pub client_x509_cert_url: Option<String>,
}

/// A type to facilitate reading and writing the json secret file
/// as returned by the [google developer console](https://code.google.com/apis/console)
#[derive(Deserialize, Serialize, Default)]
pub struct ConsoleApplicationSecret {
    pub web: Option<ApplicationSecret>,
    pub installed: Option<ApplicationSecret>,
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use hyper;

    pub const SECRET: &'static str =
        "{\"installed\":{\"auth_uri\":\"https://accounts.google.com/o/oauth2/auth\",\
         \"client_secret\":\"UqkDJd5RFwnHoiG5x5Rub8SI\",\"token_uri\":\"https://accounts.google.\
         com/o/oauth2/token\",\"client_email\":\"\",\"redirect_uris\":[\"urn:ietf:wg:oauth:2.0:\
         oob\",\"oob\"],\"client_x509_cert_url\":\"\",\"client_id\":\
         \"14070749909-vgip2f1okm7bkvajhi9jugan6126io9v.apps.googleusercontent.com\",\
         \"auth_provider_x509_cert_url\":\"https://www.googleapis.com/oauth2/v1/certs\"}}";

    #[test]
    fn console_secret() {
        use serde_json as json;
        match json::from_str::<ConsoleApplicationSecret>(SECRET) {
            Ok(s) => assert!(s.installed.is_some() && s.web.is_none()),
            Err(err) => panic!(err),
        }
    }

    #[test]
    fn schema() {
        let s = Scheme {
            token_type: TokenType::Bearer,
            access_token: "foo".to_string(),
        };
        let mut headers = hyper::header::Headers::new();
        headers.set(hyper::header::Authorization(s));
        assert_eq!(headers.to_string(),
                   "Authorization: Bearer foo\r\n".to_string());
    }

    #[test]
    fn parse_schema() {
        let auth: hyper::header::Authorization<Scheme> =
            hyper::header::Header::parse_header(&[b"Bearer foo".to_vec()]).unwrap();
        assert_eq!(auth.0.token_type, TokenType::Bearer);
        assert_eq!(auth.0.access_token, "foo".to_string());
    }
}