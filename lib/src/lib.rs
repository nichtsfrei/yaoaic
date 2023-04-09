use std::fmt::Display;

use hyper::{body::Bytes, Body, Client, Request, Uri};
use hyper_tls::HttpsConnector;

use serde::{Deserialize, Serialize};
/// A message sent to the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// The role of the message.
    pub role: String,
    /// The content of the message.
    pub content: String,
}

impl Default for Message {
    fn default() -> Self {
        Self {
            role: "user".to_string(),
            content: Default::default(),
        }
    }
}

/// The model to use.
#[derive(Default, Serialize, Clone, Deserialize)]
pub enum Model {
    /// The default model.
    #[default]
    #[serde(rename = "gpt-3.5-turbo")]
    GPT35Turbo,
    /// The code-davinci model.
    #[serde(rename = "code-davinci-002")]
    CodeDavinci,
}

impl Model {
    /// The maximum number of tokens the model can handle.
    pub fn max_tokens(&self) -> usize {
        match self {
            Model::GPT35Turbo => 4096,
            Model::CodeDavinci => 8001,
        }
    }
}

/// A query to the API.
#[derive(Clone, Deserialize, Serialize)]
#[derive(Default)]
pub struct Query {
    /// The model to use.
    pub model: Model,
    /// The messages to send.
    pub messages: Vec<Message>,
    /// The top-p value.
    pub top_p: f32,
    /// The maximum number of tokens to use.
    pub max_tokens: Option<usize>,
}



/// The usage of the API.
#[derive(Debug, Deserialize)]
pub struct Usage {
    /// The number of tokens used by the prompt.
    pub prompt_tokens: usize,
    /// The number of tokens used by the completion.
    pub completion_tokens: usize,
    /// The total number of tokens used.
    pub total_tokens: usize,
}

/// The response from the API.
#[derive(Debug, Deserialize)]
pub struct Response {
    /// The ID of the response.
    pub id: String,
    /// The object of the response.
    pub object: String,
    /// The time the response was created.
    pub created: u32,
    /// The usage of the API.
    pub usage: Usage,
    /// The choices of the response.
    pub choices: Vec<Choice>,
    /// The context of the response.
    pub context: Option<String>,
}
/// Enum representing the reasons for stopping token generation by the API.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FinishReason {
    /// The API stopped generating tokens because it reached the maximum length or because it encountered a stop token.
    Stop,
    /// The API stopped generating tokens because it exceeded the maximum time allowed for a response.
    Timeout,
    /// The API successfully generated a response and completed the prompt.
    Completion,
    /// The API encountered an error while generating a response.
    ModelError,
    /// The API encountered an error while processing the request.
    ApiError,
}

/// A choice of the response.
#[derive(Debug, Deserialize)]
pub struct Choice {
    /// The message of the choice.
    pub message: Message,
    /// The reason the choice was finished.
    pub finish_reason: FinishReason,
    /// The index of the choice.
    pub index: usize,
}

/// The API client.
pub struct OpenAIClient<'a> {
    client: Client<HttpsConnector<hyper::client::HttpConnector>>,
    api_key: &'a str,
    url: Uri,
}

#[derive(Default)]
pub enum OpenAIUri {
    #[default]
    ChatCompletion,
}

#[derive(Debug)]
pub enum Error {
    Api(ApiError),
    Unknown(String),
}

impl std::error::Error for Error {}

impl From<Box<dyn std::error::Error>> for Error {
    fn from(value: Box<dyn std::error::Error>) -> Self {
        Self::Unknown(value.to_string())
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Api(e) => write!(
                f,
                "Error response: {} {}: {}",
                e.code, e.error_type, e.message
            ),
            Error::Unknown(a) => write!(f, "{a}"),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ApiError {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: String,
}

impl OpenAIUri {
    fn as_uri(&self) -> Uri {
        match self {
            OpenAIUri::ChatCompletion => {
                match "https://api.openai.com/v1/chat/completions".parse() {
                    Ok(x) => x,
                    Err(_) => unreachable!("Hard coded uri must be parseable"),
                }
            }
        }
    }
}

impl<'a> OpenAIClient<'a> {
    /// Create a new API client.
    pub fn new(api_key: &'a str, url: OpenAIUri) -> Self {
        let https = HttpsConnector::new();
        let client = Client::builder().build(https);

        Self {
            client,
            api_key,
            url: url.as_uri(),
        }
    }

    async fn send<Q>(&self, q: Q) -> Result<Bytes, Box<dyn std::error::Error>>
    where
        Q: Serialize,
    {
        let req = Request::builder()
            .method("POST")
            .uri(self.url.clone())
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .body(Body::from(serde_json::to_string(&q)?))?;

        let res = self.client.request(req).await?;
        hyper::body::to_bytes(res.into_body())
            .await
            .map_err(|e| e.into())
    }

    /// Send a query to the API.
    pub async fn send_query(&self, q: &Query) -> Result<Response, Error> {
        let bytes = self.send(q).await?;
        serde_json::from_slice(&bytes).map_err(|e| {
            match serde_json::from_slice::<ApiError>(&bytes) {
                Ok(r) => Error::Api(r),
                Err(_) => Error::Unknown(e.to_string()),
            }
        })
    }
}
