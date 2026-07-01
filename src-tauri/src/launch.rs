use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchContext {
    pub action: Option<String>,
    pub files: Vec<String>,
}

#[derive(Default)]
pub struct PendingLaunchContexts(Mutex<Vec<LaunchContext>>);

#[tauri::command]
pub fn get_launch_context(app: AppHandle) -> LaunchContext {
    let initial = parse_launch_context(std::env::args_os().skip(1).collect());
    let state = app.state::<PendingLaunchContexts>();
    let mut pending = state.0.lock().unwrap_or_else(|error| error.into_inner());
    let mut contexts = vec![initial];
    contexts.extend(pending.drain(..));
    merge_launch_contexts(contexts)
}

pub fn store_pending_launch_context(app: &AppHandle, context: LaunchContext) {
    if context.action.is_none() || context.files.is_empty() {
        return;
    }
    let state = app.state::<PendingLaunchContexts>();
    let mut pending = state.0.lock().unwrap_or_else(|error| error.into_inner());
    pending.push(context);
}

pub fn parse_launch_context_from_strings(args: Vec<String>) -> LaunchContext {
    parse_launch_context(args.into_iter().map(OsString::from).collect())
}

pub fn parse_launch_context(args: Vec<OsString>) -> LaunchContext {
    let mut action = None;
    let mut files = Vec::new();
    let mut consume_files = false;
    let mut iter = args.into_iter().peekable();

    while let Some(arg) = iter.next() {
        let text = arg.to_string_lossy().to_string();
        match text.as_str() {
            "--pwc-action" => {
                if let Some(value) = iter.next() {
                    action = normalize_action(&value.to_string_lossy());
                }
                consume_files = false;
            }
            "--files" => {
                consume_files = true;
            }
            flag if flag.starts_with("--") => {
                consume_files = false;
            }
            _ => {
                if consume_files || action.is_some() {
                    files.push(text);
                }
            }
        }
    }

    LaunchContext {
        action,
        files: unique_files(files),
    }
}

fn normalize_action(value: &str) -> Option<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "encode" | "decode" | "scan" => Some(value.trim().to_ascii_lowercase()),
        _ => None,
    }
}

fn merge_launch_contexts(contexts: Vec<LaunchContext>) -> LaunchContext {
    let action = contexts.iter().rev().find_map(|context| context.action.clone());
    let Some(action) = action else {
        return LaunchContext {
            action: None,
            files: Vec::new(),
        };
    };

    let files = contexts
        .into_iter()
        .filter(|context| context.action.as_deref() == Some(action.as_str()))
        .flat_map(|context| context.files)
        .collect::<Vec<_>>();

    LaunchContext {
        action: Some(action),
        files: unique_files(files),
    }
}

fn unique_files(files: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut output = Vec::new();

    for file in files {
        let trimmed = file.trim().trim_matches('"').to_owned();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_ascii_lowercase();
        if seen.insert(key) {
            output.push(trimmed);
        }
    }

    output
}
