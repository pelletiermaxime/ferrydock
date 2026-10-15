//! Interactive Docker PTY, prompt integration, and guarded favorite dispatch.
use super::{Event, Shell};
use anyhow::{Context, Result, bail};
use bollard::{
    Docker,
    exec::{CreateExecOptions, ResizeExecOptions, StartExecOptions, StartExecResults},
};
use futures_util::StreamExt;
use tokio::{io::AsyncWriteExt, sync::mpsc};
use tokio_util::sync::CancellationToken;

pub(super) enum TerminalCommand {
    Input(Vec<u8>),
    Reply(Vec<u8>),
    Resize(u16, u16),
    Favorite {
        run: u64,
        target: crate::favorites::Target,
        command: String,
    },
}

// Only our session's OSC is consumed. Retain at most a prefix plus 32 bytes,
// including when output splits an escape sequence across Docker frames.
struct PromptDecoder {
    prefix: Vec<u8>,
    pending: Vec<u8>,
}
enum Part {
    Data(Vec<u8>),
    Prompt(i64),
}
impl PromptDecoder {
    fn new(token: &str) -> Self {
        Self {
            prefix: format!("\x1b]777;ferrydock;{token};").into_bytes(),
            pending: vec![],
        }
    }
    fn feed(&mut self, data: &[u8]) -> Vec<Part> {
        self.pending.extend_from_slice(data);
        let mut parts = Vec::new();
        loop {
            if let Some(start) = self
                .pending
                .windows(self.prefix.len())
                .position(|w| w == self.prefix)
            {
                if start > 0 {
                    parts.push(Part::Data(self.pending.drain(..start).collect()));
                }
                if let Some(end) = self.pending[self.prefix.len()..]
                    .iter()
                    .position(|b| *b == 7)
                {
                    let end = end + self.prefix.len();
                    let code = std::str::from_utf8(&self.pending[self.prefix.len()..end])
                        .ok()
                        .and_then(|s| s.parse::<i64>().ok());
                    let bytes: Vec<_> = self.pending.drain(..=end).collect();
                    if let Some(code) = code.filter(|code| (0..=255).contains(code)) {
                        parts.push(Part::Prompt(code));
                    } else {
                        parts.push(Part::Data(bytes));
                    }
                } else if self.pending.len() > self.prefix.len() + 32 {
                    parts.push(Part::Data(self.pending.drain(..1).collect()));
                } else {
                    break;
                }
            } else {
                let retained = (1..self.prefix.len())
                    .rev()
                    .find(|n| self.pending.ends_with(&self.prefix[..*n]))
                    .unwrap_or(0);
                let count = self.pending.len() - retained;
                if count > 0 {
                    parts.push(Part::Data(self.pending.drain(..count).collect()));
                }
                break;
            }
        }
        parts
    }
}

const SH_SETUP: &str = r#"PS1='$(printf "\033]777;ferrydock;%s;%s\007" "$FERRYDOCK_PROMPT_TOKEN" "$?")'"${PS1:-\$ }"; export PS1; exec sh -i"#;
// Load the normal user startup file before composing the prompt hook. Supplying
// PROMPT_COMMAND as an exec environment variable lets .bashrc overwrite it.
// The rc file is an inherited pipe; no files are written inside the container.
const BASH_BOOTSTRAP: &str = r#"exec bash --rcfile <(printf '%s\n' "$1") -i"#;
const BASH_INIT: &str = r#"
if [[ -r ~/.bashrc ]]; then source ~/.bashrc; fi
__ferrydock_prompt() {
    local __ferrydock_exit=$?
    printf '\033]777;ferrydock;%s;%s\007' "$FERRYDOCK_PROMPT_TOKEN" "$__ferrydock_exit"
    return "$__ferrydock_exit"
}
case "$(declare -p PROMPT_COMMAND 2>/dev/null)" in
    'declare -a'*) PROMPT_COMMAND=(__ferrydock_prompt "${PROMPT_COMMAND[@]}") ;;
    *) PROMPT_COMMAND="__ferrydock_prompt${PROMPT_COMMAND:+; $PROMPT_COMMAND}" ;;
esac
"#;
fn shell_command(shell: Shell) -> Vec<String> {
    match shell {
        Shell::Auto => vec![
            "sh".into(),
            "-c".into(),
            format!(
                "if command -v bash >/dev/null 2>&1; then exec bash -c \"$1\" ferrydock-bootstrap \"$2\"; else {SH_SETUP}; fi"
            ),
            "ferrydock-auto".into(),
            BASH_BOOTSTRAP.into(),
            BASH_INIT.into(),
        ],
        Shell::Bash => vec![
            "bash".into(),
            "-c".into(),
            BASH_BOOTSTRAP.into(),
            "ferrydock-bootstrap".into(),
            BASH_INIT.into(),
        ],
        Shell::Sh => vec!["sh".into(), "-c".into(), SH_SETUP.into()],
    }
}

fn command_input(command: &str) -> Vec<u8> {
    // A multiline recipe is one compound command and produces one final prompt.
    // No subshell: cd/export and shell state behave like commands typed by hand.
    if command.contains('\n') {
        format!("{{\r{}\r}}\r", command.replace('\n', "\r")).into_bytes()
    } else {
        format!("{command}\r").into_bytes()
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn terminal_session(
    docker: Docker,
    id: String,
    generation: u64,
    shell: Shell,
    columns: u16,
    rows: u16,
    mut commands: mpsc::UnboundedReceiver<TerminalCommand>,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
) {
    let mut favorite = None;
    let mut exit_code = None;
    let result = async {
        let token = format!(
            "{}-{generation}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        );
        let exec = docker
            .create_exec(
                &id,
                CreateExecOptions {
                    attach_stdin: Some(true),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    tty: Some(true),
                    cmd: Some(shell_command(shell)),
                    env: Some(vec![
                        "TERM=xterm-256color".into(),
                        "COLORTERM=truecolor".into(),
                        format!("FERRYDOCK_PROMPT_TOKEN={token}"),
                    ]),
                    ..Default::default()
                },
            )
            .await
            .context(
                "Cannot open shell. Try sh if Bash is unavailable; this image may have no shell",
            )?;
        let StartExecResults::Attached {
            mut output,
            mut input,
        } = docker
            .start_exec(
                &exec.id,
                Some(StartExecOptions {
                    detach: false,
                    tty: true,
                    ..Default::default()
                }),
            )
            .await?
        else {
            bail!("Docker did not attach the terminal");
        };
        docker
            .resize_exec(
                &exec.id,
                ResizeExecOptions {
                    height: rows,
                    width: columns,
                },
            )
            .await?;
        let _ = events.send(Event::TerminalReady { generation }).await;
        let mut decoder = PromptDecoder::new(&token);
        let mut at_prompt = false;
        let mut seen_prompt = false;
        let mut submitted = false;
        let mut input_count = 0;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => { let _ = input.shutdown().await; break; },
                frame = output.next() => match frame {
                    Some(Ok(frame)) => for part in decoder.feed(&frame.into_bytes()) {
                        match part {
                            Part::Data(data) => { events.send(Event::TerminalData { generation, data }).await?; }
                            Part::Prompt(code) => {
                                if let Some(run) = favorite.take() { events.send(Event::FavoriteFinished { generation, run, result: Ok(code) }).await?; }
                                at_prompt = at_prompt || if seen_prompt { input_count == 0 || submitted } else { input_count == 0 };
                                seen_prompt = true;
                                submitted = false;
                                events.send(Event::TerminalPrompt { generation, input_count, at_prompt }).await?;
                            }
                        }
                    },
                    Some(Err(error)) => return Err(error.into()),
                    None => {
                        exit_code = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                            loop {
                                let info = docker.inspect_exec(&exec.id).await.ok()?;
                                if info.running != Some(true) && info.exit_code.is_some() { return info.exit_code; }
                                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                            }
                        }).await.ok().flatten();
                        break;
                    },
                },
                command = commands.recv() => match command {
                    Some(TerminalCommand::Input(data)) => {
                        input_count += 1;
                        at_prompt = false;
                        submitted = matches!(data.last(), Some(b'\r' | b'\n' | 3));
                        input.write_all(&data).await?; input.flush().await?;
                    },
                    Some(TerminalCommand::Reply(data)) => { input.write_all(&data).await?; input.flush().await?; },
                    Some(TerminalCommand::Resize(columns, rows)) => { docker.resize_exec(&exec.id, ResizeExecOptions { height: rows, width: columns }).await?; },
                    Some(TerminalCommand::Favorite { run, target, command }) => {
                        if !at_prompt || favorite.is_some() {
                            events.send(Event::FavoriteFinished { generation, run, result: Err("Command not sent: the terminal is running a command or has unfinished input".into()) }).await?;
                            continue;
                        }
                        let valid = tokio::time::timeout(std::time::Duration::from_secs(5), docker.inspect_container(&id, None)).await.ok().and_then(Result::ok).is_some_and(|info| {
                            let labels = info.config.and_then(|c| c.labels).unwrap_or_default();
                            let matches = match target {
                                crate::favorites::Target::Service { project, service } => labels.get("com.docker.compose.project") == Some(&project) && labels.get("com.docker.compose.service") == Some(&service),
                                crate::favorites::Target::Container { name } => info.name.as_deref().map(|n| n.trim_start_matches('/')) == Some(name.as_str()) && !labels.contains_key("com.docker.compose.project"),
                            };
                            matches && info.state.and_then(|s| s.running) == Some(true)
                        });
                        if !valid || cancel.is_cancelled() {
                            events.send(Event::FavoriteFinished { generation, run, result: Err("Command not sent: container is stopped, absent or unavailable".into()) }).await?;
                            continue;
                        }
                        at_prompt = false; submitted = true; favorite = Some(run);
                        input.write_all(&command_input(&command)).await?; input.flush().await?;
                        events.send(Event::FavoriteStarted { generation, run }).await?;
                    },
                    None => { let _ = input.shutdown().await; break; },
                }
            }
        }
        if !decoder.pending.is_empty() {
            let _ = events
                .send(Event::TerminalData {
                    generation,
                    data: decoder.pending,
                })
                .await;
        }
        Ok::<_, anyhow::Error>(())
    };
    let error = tokio::select! { _ = cancel.cancelled() => None, result = result => result.err().map(|error| error.to_string()) };
    if let Some(run) = favorite {
        let result = exit_code.filter(|_| !cancel.is_cancelled()).ok_or_else(|| {
            error
                .clone()
                .unwrap_or_else(|| "Session closed; command completion is unknown".into())
        });
        let _ = events
            .send(Event::FavoriteFinished {
                generation,
                run,
                result,
            })
            .await;
    }
    let _ = events
        .send(Event::TerminalEnded { generation, error })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_prompts_are_hidden_without_losing_output_or_exit_codes() {
        let bytes = b"before\x1b]777;ferrydock;test;17\x07after";
        for split in 0..=bytes.len() {
            let mut decoder = PromptDecoder::new("test");
            let parts = decoder
                .feed(&bytes[..split])
                .into_iter()
                .chain(decoder.feed(&bytes[split..]));
            let mut output = Vec::new();
            let mut codes = vec![];
            for part in parts {
                match part {
                    Part::Data(data) => output.extend(data),
                    Part::Prompt(code) => codes.push(code),
                }
            }
            assert_eq!(output, b"beforeafter");
            assert_eq!(codes, vec![17]);
            assert!(decoder.pending.is_empty());
        }
        let mut decoder = PromptDecoder::new("test");
        let junk = format!("\x1b]777;ferrydock;test;{}", "x".repeat(4096));
        decoder.feed(junk.as_bytes());
        assert!(decoder.pending.len() <= decoder.prefix.len() + 32);
    }
    #[test]
    fn unrelated_and_malformed_escapes_keep_their_original_bytes() {
        let data = b"\x1b[32mgreen\x1b[0m \x1b]777;ferrydock;another;0\x07 \x1b]777;ferrydock;test;bad\x07";
        let mut decoder = PromptDecoder::new("test");
        let mut result = vec![];
        for byte in data {
            for part in decoder.feed(&[*byte]) {
                match part {
                    Part::Data(bytes) => result.extend(bytes),
                    Part::Prompt(_) => panic!("Not a valid session prompt"),
                }
            }
        }
        result.extend(decoder.pending);
        assert_eq!(result, data);
    }
    #[test]
    fn multiline_recipes_are_one_shell_group() {
        assert_eq!(command_input("composer i"), b"composer i\r");
        assert_eq!(
            command_input("cd /tmp\nprintf ok"),
            b"{\rcd /tmp\rprintf ok\r}\r"
        );
    }
}
