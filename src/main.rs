use clap::{Parser, Subcommand, ValueEnum};
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

use plan_to_git::capture;
use plan_to_git::codex_history::{self, CodexHistoryImportOutcome};
use plan_to_git::error::{AppError, AppResult};
use plan_to_git::git;
use plan_to_git::github::{self, SyncStatus};
use plan_to_git::render::render_plan_comment;
use plan_to_git::store::{load_state, save_state, STATE_FILE_NAME};

#[derive(Parser, Debug)]
#[command(
    name = "plan-to-git",
    about = "Capture agent plans and sync them to GitHub pull requests"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Process an agent hook JSON payload from stdin.
    Hook {
        #[arg(long, value_enum)]
        source: HookSource,
    },
    /// Import explicitly marked plans from previous Codex session files.
    #[command(visible_alias = "backfill-codex")]
    ImportCodex {
        /// Codex home directory. Defaults to `CODEX_HOME` or `~/.codex`.
        #[arg(long)]
        codex_home: Option<PathBuf>,
        /// Scan and report what would be imported without writing or syncing.
        #[arg(long)]
        dry_run: bool,
        /// Save imported plans locally without posting a pull request comment.
        #[arg(long)]
        no_sync: bool,
    },
    /// Post newly captured plan items to the current branch pull request.
    Sync,
    /// Print the local plan stack JSON.
    Show,
    /// Render the local plan stack markdown.
    Render,
    /// Clear local plan stack state.
    Clear {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum HookSource {
    Codex,
}

fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Hook { source } => {
            if let Err(error) = run_hook(*source) {
                eprintln!("plan-to-git hook error: {error}");
            }
        }
        command => {
            if let Err(error) = run(command) {
                eprintln!("plan-to-git error: {error}");
                std::process::exit(1);
            }
        }
    }
}

fn run(command: &Commands) -> AppResult<()> {
    match command {
        Commands::Hook { .. } => Ok(()),
        Commands::ImportCodex {
            codex_home,
            dry_run,
            no_sync,
        } => {
            let (context, state_path) = state_context()?;
            let mut state = load_state(&state_path)?;
            state.set_context(
                context.repo_slug.clone(),
                context.branch.clone(),
                context.head_sha.clone(),
            );

            let codex_home = codex_home
                .clone()
                .or_else(default_codex_home)
                .ok_or_else(|| AppError::new("cannot locate Codex home directory"))?;
            let outcome = codex_history::import_codex_history(&codex_home, &context, &mut state)?;

            if !*dry_run && outcome.plans_added > 0 {
                save_state(&state_path, &state)?;
            }

            print_import_outcome(&outcome, *dry_run);

            if *dry_run || *no_sync || outcome.plans_found == 0 {
                return Ok(());
            }

            let sync_status = github::sync_state(&context, &mut state)?;
            save_state(&state_path, &state)?;
            print_sync_status(&sync_status);
            Ok(())
        }
        Commands::Sync => {
            let (context, state_path) = state_context()?;
            let mut state = load_state(&state_path)?;
            state.set_context(
                context.repo_slug.clone(),
                context.branch.clone(),
                context.head_sha.clone(),
            );
            let sync_status = github::sync_state(&context, &mut state)?;
            save_state(&state_path, &state)?;
            print_sync_status(&sync_status);
            Ok(())
        }
        Commands::Show => {
            let (_, state_path) = state_context()?;
            let state = load_state(&state_path)?;
            println!("{}", serde_json::to_string_pretty(&state)?);
            Ok(())
        }
        Commands::Render => {
            let (_, state_path) = state_context()?;
            let state = load_state(&state_path)?;
            println!(
                "{}",
                render_plan_comment(&state, &state.unposted_items_for_pr(0))
            );
            Ok(())
        }
        Commands::Clear { yes } => {
            if !*yes {
                return Err(AppError::new("refusing to clear state without --yes").into());
            }
            let (_, state_path) = state_context()?;
            if state_path.exists() {
                fs::remove_file(state_path)?;
            }
            println!("cleared {STATE_FILE_NAME}");
            Ok(())
        }
    }
}

fn run_hook(source: HookSource) -> AppResult<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    match source {
        HookSource::Codex => {
            let outcome = capture::process_codex_hook(&input)?;
            eprintln!(
                "plan-to-git: captured {} plan(s), {} decision(s), {} pending question set(s), sync={:?}",
                outcome.captured_plans,
                outcome.captured_decisions,
                outcome.pending_questions,
                outcome.sync_status
            );
        }
    }

    Ok(())
}

fn state_context() -> AppResult<(git::GitContext, std::path::PathBuf)> {
    let cwd = std::env::current_dir()?;
    let context = git::discover(&cwd)?;
    let state_path = context.repo_root.join(STATE_FILE_NAME);
    Ok((context, state_path))
}

fn print_sync_status(status: &SyncStatus) {
    match status {
        SyncStatus::NoItems => println!("no captured plan items to sync"),
        SyncStatus::NoPullRequest => println!("no pull request found for the current branch"),
        SyncStatus::ClosedPullRequest { number, state } => {
            println!("pull request #{number} is {state}; leaving plan items queued");
        }
        SyncStatus::DraftPullRequest { number } => {
            println!("pull request #{number} is a draft; leaving plan items queued");
        }
        SyncStatus::Unchanged { number } => {
            println!("no new plan items to comment on pull request #{number}");
        }
        SyncStatus::Commented {
            number,
            comment_id,
            items,
        } => {
            println!("posted {items} plan item(s) to pull request #{number} comment #{comment_id}");
        }
    }
}

fn default_codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
}

fn print_import_outcome(outcome: &CodexHistoryImportOutcome, dry_run: bool) {
    let mode = if dry_run { "dry-run" } else { "import" };
    println!(
        "{mode}: scanned {} file(s), matched {} current repo/branch file(s), found {} plan(s), added {}, skipped {} duplicate(s), skipped {} rendered stack(s), parse errors {}",
        outcome.files_scanned,
        outcome.files_matched,
        outcome.plans_found,
        outcome.plans_added,
        outcome.duplicates,
        outcome.rendered_stacks_skipped,
        outcome.parse_errors
    );
}
