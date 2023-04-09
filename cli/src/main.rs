use std::{
    fs::{self, OpenOptions},
    future::Future,
    io::prelude::Write,
    io::SeekFrom,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use yaoaic::{Message, OpenAIClient, Query};

use anyhow::{bail, Context, Result};
use std::io::Seek;
#[derive(Default, Clone, ValueEnum)]
enum Model {
    /// The default model.
    #[default]
    GPT35Turbo,
    /// The code-davinci model.
    CodeDavinci,
}

impl Model {
    fn as_yaoic_model(&self) -> yaoaic::Model {
        match self {
            Model::GPT35Turbo => yaoaic::Model::GPT35Turbo,
            Model::CodeDavinci => yaoaic::Model::CodeDavinci,
        }
    }
}

const DEFAULT_SOURCES: &[prompts::Source] = &[
    prompts::Source::Http(
        "https://raw.githubusercontent.com/f/awesome-chatgpt-prompts/main/prompts.csv",
    ),
    //prompts::Source::File("~/.local/cache/yaoaic/prompts.csv"),
];

struct Cache<'a> {
    dir: &'a str,
    cache: bool,
    max_cache_age: Duration,
}

impl<'a> Cache<'a> {
    fn check_or_create(&self) -> Result<()> {
        if let Ok(exist) = fs::metadata(self.dir) {
            if !exist.is_dir() {
                bail!("{} exists but it is not a dir.", self.dir);
            }
            Ok(())
        } else {
            fs::create_dir(self.dir).with_context(|| format!("unable to create dir {}", self.dir))
        }
    }

    fn load_cached<T>(&self, file_name: &str) -> Result<Option<T>>
    where
        T: Serialize + serde::de::DeserializeOwned,
    {
        if !self.cache {
            return Ok(None);
        }
        self.check_or_create()?;
        let cached = fs::read_to_string(format!("{}/{file_name}", self.dir))
            .context("unable to load prompts.toml")?;
        let cached: CachedValue<T> =
            toml::from_str(&cached).context("prompts.toml has unknown format.")?;

        let created = cached.created;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
        if now - created < self.max_cache_age {
            Ok(Some(cached.value))
        } else {
            Ok(None)
        }
    }

    fn store_cache<T>(&self, file_name: &str, to_cache: T) -> Result<()>
    where
        T: serde::ser::Serialize,
    {
        self.check_or_create()?;
        let cached_toml =
            toml::to_string_pretty(&to_cache).context("unable to wrote cached prompts toml")?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(format!("{}/{file_name}", self.dir))
            .context("unable to open or create prompts.toml")?;
        file.seek(SeekFrom::Start(0))
            .context("unable to seek to first byte")?;
        file.write_all(cached_toml.as_bytes())
            .context("unable to write cached prompts")
    }

    pub async fn with_cached<F, T, I>(
        &self,
        file_name: &str,
        input: I,
        mut loader: impl FnMut(I) -> F,
    ) -> Result<T>
    where
        T: Serialize + serde::de::DeserializeOwned + Sized,
        F: Future<Output = Result<T>>,
    {
        match self.load_cached::<T>("prompts.toml") {
            Ok(Some(x)) => Ok(x),
            Ok(None) | Err(_) => {
                let r = loader(input).await?;
                let cached: CachedValue<T> = r.into();
                if self.cache {
                    self.store_cache(file_name, &cached)?;
                }
                Ok(cached.value)
            }
        }
    }
}

pub async fn valid_prompts<'a>(sources: &[prompts::Source<'a>]) -> Result<Vec<prompts::Prompt>> {
    let results = prompts::PromptLoader::load(sources).await;
    Ok(results.into_iter().filter_map(|e| e.ok()).collect())
}

//#[derive(Default, Serialize, Clone, Deserialize, ValueEnum)]
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long, value_enum)]
    model: Option<Model>,
    #[arg(long, default_value_t = 0.5)]
    top_p: f32,
    #[arg(short, long)]
    max_tokens: Option<usize>,

    #[arg(long, default_value_t = true)]
    /// Enable or disable cache
    cache: bool,
    /// Sets the amount of seconds that a cache is valid (default 86400s or 24h.)
    #[arg(long, default_value_t = 60 * 60 * 24 * 1)]
    cache_timeout_second: u64,

    #[arg(short, long)]
    prompt: Option<String>,

    #[arg(short, long, action = clap::ArgAction::SetTrue)]
    stdin: bool,
    /// when no stdin is given, fallback to the file
    input_file: Option<String>,
    #[command(subcommand)]
    cmd: Option<AdditionalCmd>,
}

#[derive(Subcommand)]
enum AdditionalCmd {
    /// Adds files to myapp
    Prompt {
        #[command(subcommand)]
        cmd: PromptCommands,
    },
}

#[derive(Subcommand)]
enum PromptCommands {
    List {
        filter: Option<String>,
    },
    Select {
        /// when no stdin is given, fallback to the file
        option: String,
    },
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
struct CachedValue<T>
where
    T: serde::Serialize,
{
    created: Duration,
    value: T,
}

impl<T> From<T> for CachedValue<T>
where
    T: serde::Serialize,
{
    fn from(value: T) -> Self {
        let start = SystemTime::now();
        let created = start
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards");
        Self { created, value }
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();
    let cache_dir = format!("{}/.local/share/yaoaic", env!("HOME"));
    let cache = Cache {
        cache: args.cache,
        dir: &cache_dir,
        max_cache_age: Duration::new(args.cache_timeout_second, 0),
    };

    let api_key = env!("OPENAI_API_KEY");
    let client = OpenAIClient::new(api_key, Default::default());
    let mut messages = vec![];
    match args.cmd {
        Some(AdditionalCmd::Prompt { cmd }) => match cmd {
            PromptCommands::List { filter } => {
                let cached = cache
                    .with_cached("prompts.toml", DEFAULT_SOURCES, valid_prompts)
                    .await?;
                let filter = filter.map(|e| e.to_lowercase()).unwrap_or_default();
                for (i, p) in cached.iter().enumerate() {
                    if p.act.to_lowercase().contains(&filter)
                        || p.prompt.to_lowercase().contains(&filter)
                    {
                        println!("{i}: {}", p.act);
                    }
                }
                return Ok(());
            }
            PromptCommands::Select { option } => {
                let prmpts = cache
                    .with_cached("prompts.toml", DEFAULT_SOURCES, valid_prompts)
                    .await?;
                let indexed: Option<usize> = match option.parse::<usize>() {
                    Ok(x) => Some(x),
                    Err(_) => None,
                };
                let prompt = prmpts.into_iter().enumerate().find(|(i, p)| {
                    if let Some(wi) = indexed {
                        wi == *i
                    } else {
                        p.act == option
                    }
                });
                if let Some((i, p)) = prompt {
                    eprintln!("using {i}: {}", p.act);

                    let cfn = format!("{i}_messages.toml");
                    let mut prompt_msg = Message::default();
                    prompt_msg.content = p.prompt;
                    let mut q = Query::default();
                    q.messages = vec![prompt_msg];
                    if let Ok(Some(r)) = cache.load_cached::<Vec<Message>>(&cfn) {
                        messages.extend(r)
                    } else {
                        let response = client.send_query(&q).await?;
                        messages.extend(q.messages);
                        messages.extend(response.choices.into_iter().map(|c| c.message));
                        if cache.cache {
                            let to_cache: CachedValue<Vec<Message>> = messages.clone().into();
                            cache.store_cache(&cfn, to_cache)?;
                        }
                    }
                }
            }
        },
        None => {}
    };

    let input = {
        if args.stdin {
            std::io::stdin()
                .lines()
                .filter_map(|e| e.ok())
                .collect::<Vec<String>>()
                .join("")
        } else {
            fs::read_to_string(args.input_file.unwrap_or_default())
                .context("unable to load file")?
        }
    };
    messages.push(Message {
        content: input.trim().to_owned(),
        ..Default::default()
    });
    let q = Query {
        model: args.model.unwrap_or_default().as_yaoic_model(),
        top_p: args.top_p,
        max_tokens: args.max_tokens,

        messages,
    };

    let response = client.send_query(&q).await?;
    if let Some(r) = response.choices.first() {
        println!("{}", r.message.content)
    }

    Ok(())
}
