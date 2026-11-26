use ametller_mcp::{
    Error, Result,
    api::validate_locale,
    mcp,
    service::App,
    store::{CredentialStore, Store},
};
use clap::{Parser, Subcommand};
use serde_json::Value;
use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
};
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(
    version,
    about = "Ametller catalog and cart MCP. Login secrets are entered privately in a terminal."
)]
struct Cli {
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
    #[arg(long, global = true, value_enum)]
    #[cfg_attr(target_os = "macos", arg(default_value = "keychain"))]
    #[cfg_attr(not(target_os = "macos"), arg(default_value = "file"))]
    credential_store: CredentialStore,
    #[arg(long,global=true,default_value="es",value_parser=["es","ca"])]
    locale: String,
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    /// Run the stdio MCP server (default); no network or credentials at startup.
    Serve,
    /// Print offline capabilities.
    Capabilities,
    /// Sign in directly with terminal-only email and hidden password prompts.
    Login {
        /// Compatibility flag; all login commands now use the terminal prompt.
        #[arg(long, hide = true)]
        password: bool,
    },
    /// Delete local credentials; does not remotely revoke the session.
    Logout,
    /// Read cached connection metadata, or validate with the shop using --live.
    Status {
        #[arg(long)]
        live: bool,
    },
    Search {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: u32,
    },
    Product {
        product_id: String,
    },
    /// Synchronize online orders into the private SQLite cache; never changes orders.
    SyncHistory {
        #[arg(long, default_value_t = 5)]
        max_pages: u32,
        /// Start again from the newest page instead of resuming incomplete work.
        #[arg(long)]
        restart: bool,
    },
    /// Read cached orders without contacting Ametller or reading Keychain secrets.
    History {
        #[arg(long)]
        order_id: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: u32,
    },
    /// Find products in cached non-cancelled orders, for repeat-shopping suggestions.
    PurchasedProducts {
        #[arg(default_value = "")]
        query: String,
        #[arg(long, default_value = "frequent", value_parser=["frequent","recent"])]
        sort: String,
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: u32,
    },
    /// Delete only this connected account's local order cache; remote orders stay intact.
    ClearHistory,
    /// Read the account cart; never creates a basket.
    Cart {
        /// Manually unblock an uncertain write after inspecting the official cart.
        #[arg(long)]
        clear_uncertain: bool,
    },
    /// Set desired absolute quantity in purchase units; zero removes.
    SetCartItem {
        product_id: String,
        quantity: String,
        #[arg(long)]
        item_id: Option<String>,
    },
}
fn print(value: Value) -> Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, &value)
        .map_err(|_| Error::Input("Could not write output."))?;
    writeln!(out).map_err(|_| Error::Input("Could not write output."))
}
fn credentials() -> Result<(Zeroizing<String>, Zeroizing<String>)> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(Error::Input(
            "login requires an interactive terminal. Never send your password in chat, arguments, files or environment variables.",
        ));
    }
    eprint!("Ametller email: ");
    io::stderr()
        .flush()
        .map_err(|_| Error::Input("Could not open terminal."))?;
    let mut email = Zeroizing::new(String::new());
    io::stdin()
        .read_line(&mut email)
        .map_err(|_| Error::Input("Could not read terminal."))?;
    let email = Zeroizing::new(email.trim().to_owned());
    let password = Zeroizing::new(
        rpassword::prompt_password("Ametller password (hidden): ")
            .map_err(|_| Error::Input("Could not read private password."))?,
    );
    Ok((email, password))
}
async fn run() -> Result<()> {
    let cli = Cli::parse();
    if matches!(cli.command, Some(Command::Capabilities)) {
        return print(mcp::capabilities());
    }
    validate_locale(&cli.locale)?;
    let app = App::new(
        Store {
            directory: cli
                .state_dir
                .map(Ok)
                .unwrap_or_else(Store::default_directory)?,
            backend: cli.credential_store,
        },
        cli.locale,
    )?;
    let output = match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => return mcp::serve(app).await,
        Command::Capabilities => unreachable!(),
        Command::Login { .. } => {
            let (email, password) = credentials()?;
            app.login(&email, &password).await?
        }
        Command::Logout => app.logout().await?,
        Command::Status { live } => app.status(live).await?,
        Command::Search {
            query,
            limit,
            offset,
        } => app.search(&query, limit, offset).await?,
        Command::Product { product_id } => app.product(&product_id).await?,
        Command::SyncHistory { max_pages, restart } => app.sync_history(max_pages, restart).await?,
        Command::History {
            order_id,
            limit,
            offset,
        } => {
            app.purchase_history(order_id.as_deref(), limit, offset)
                .await?
        }
        Command::PurchasedProducts {
            query,
            sort,
            limit,
            offset,
        } => app.purchased_products(&query, &sort, limit, offset).await?,
        Command::ClearHistory => app.clear_history().await?,
        Command::Cart { clear_uncertain } => app.cart(clear_uncertain).await?,
        Command::SetCartItem {
            product_id,
            quantity,
            item_id,
        } => {
            app.set_cart_item(&product_id, &quantity, item_id.as_deref())
                .await?
        }
    };
    print(output)
}
#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("ametller-mcp: {error}");
        std::process::exit(1);
    }
}
