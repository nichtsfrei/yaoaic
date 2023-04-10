use std::{fs, time::Duration};

use clap::{Parser, Subcommand, ValueEnum};

use yaoaic::{Message, OpenAIClient, Query};

use anyhow::{Context, Result};

mod cache;
mod toml_file;
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

pub async fn valid_prompts<'a>(sources: &[prompts::Source<'a>]) -> Result<Vec<prompts::Prompt>> {
    let results = prompts::PromptLoader::load(sources).await;
    let mut only_ok = Vec::with_capacity(results.len());
    for r in results {
        match r {
            Ok(r) => only_ok.push(r),
            Err(e) => eprintln!("warning: {e}"),
        }
    }
    Ok(only_ok)
}

pub async fn ask<'a>(query_client: (&'a Query, &'a OpenAIClient<'a>)) -> Result<Vec<Message>> {
    let (q, client) = query_client;
    let mut messages = q.messages.clone();

    let response = client.send_query(&q).await?;
    messages.extend(response.choices.into_iter().map(|c| c.message));
    Ok(messages)
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

#[tokio::main]
async fn main() -> Result<()> {
    let user_prompts = format!("{}/.config/yaoaic/prompts.csv", env!("HOME"));
    let sources: &[prompts::Source] = &[
        prompts::Source::Http(
             "https://raw.githubusercontent.com/f/awesome-chatgpt-prompts/main/prompts.csv",
        ),
        prompts::Source::File(&user_prompts),
        //prompts::Source::File("~/.local/cache/yaoaic/prompts.csv"),
    ];
    let args = Cli::parse();
    let cache_dir = format!("{}/.local/share/yaoaic", env!("HOME"));

    let c = {
        if args.cache {
            Some(cache::init(
                &cache_dir,
                Duration::new(args.cache_timeout_second, 0),
            )?)
        } else {
            None
        }
    };
    let api_key = env!("OPENAI_API_KEY");
    let client = OpenAIClient::new(api_key, Default::default());
    let mut messages: Vec<Message> = vec![];
    match args.cmd {
        Some(AdditionalCmd::Prompt { cmd }) => {
            let all_prompts = {
                match &c {
                    Some(c) => c.with_cached("prompts.toml", sources, valid_prompts).await,
                    None => valid_prompts(sources).await,
                }?
            };
            match cmd {
                PromptCommands::List { filter } => {
                    let filter = filter.map(|e| e.to_lowercase()).unwrap_or_default();
                    for (i, p) in all_prompts.iter().enumerate() {
                        if p.act.to_lowercase().contains(&filter)
                            || p.prompt.to_lowercase().contains(&filter)
                        {
                            println!("{i}: {}", p.act);
                        }
                    }
                    return Ok(());
                }
                PromptCommands::Select { option } => {
                    let indexed: Option<usize> = match option.parse::<usize>() {
                        Ok(x) => Some(x),
                        Err(_) => None,
                    };
                    let prompt = all_prompts.into_iter().enumerate().find(|(i, p)| {
                        if let Some(wi) = indexed {
                            wi == *i
                        } else {
                            p.act == option
                        }
                    });
                    if let Some((i, p)) = prompt {
                        let cfn = format!("{i}_messages.toml");
                        let mut prompt_msg = Message::default();
                        prompt_msg.content = p.prompt;
                        let mut q = Query::default();
                        q.messages = vec![prompt_msg];
                        let r = match &c {
                            Some(c) => c.with_cached(&cfn, (&q, &client), ask).await,
                            None => ask((&q, &client)).await,
                        }?;
                        messages.extend(r);
                    }
                }
            }
        }
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
    if let Some(c) = c {
        let mut cache_messages = q.messages.clone();
        cache_messages.extend(response.choices.into_iter().map(|c| c.message));
        let cached: cache::Value<Vec<Message>> = cache_messages.into();
        c.store_cache("last_messages.toml", cached).await?;
    }

    Ok(())
}
