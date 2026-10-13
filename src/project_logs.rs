//! Merge Docker log streams for the selected project's current membership.
use crate::{
    ansi::{Decoder, StyledLine},
    docker::Event,
    model::{Container, LogBuffer},
};
use bollard::{Docker, query_parameters::LogsOptionsBuilder};
use chrono::{DateTime, FixedOffset};
use futures_util::{StreamExt, stream::FuturesUnordered};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    time::Duration,
};
use tokio::{
    sync::{mpsc, watch},
    time,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct Line {
    pub container: String,
    pub service: String,
    pub name: String,
    pub text: String,
    pub error: bool,
}
impl Line {
    pub fn display(&self) -> String {
        format!("[{} / {}] {}", self.service, self.name, self.text)
    }
}

pub struct Record {
    line: Line,
    pub styled: StyledLine,
    prefix: String,
}

impl std::ops::Deref for Record {
    type Target = Line;
    fn deref(&self) -> &Line {
        &self.line
    }
}

impl Record {
    pub fn render(&self, fallback: u32) -> gpui::StyledText {
        self.styled.render(&self.prefix, fallback)
    }

    pub fn render_on(&self, fallback: u32, background: u32) -> gpui::StyledText {
        self.styled.render_on(&self.prefix, fallback, background)
    }

    pub fn render_source_on(
        &self,
        source_color: u32,
        fallback: u32,
        background: u32,
    ) -> gpui::StyledText {
        self.styled
            .render_with_source_on(&self.prefix, source_color, fallback, background)
    }
}

#[derive(Default)]
pub struct History {
    pub lines: VecDeque<Record>,
    services: BTreeMap<String, usize>,
    decoders: HashMap<String, Decoder>,
}
impl History {
    pub fn push(&mut self, mut line: Line) {
        let (_, body) = crate::ansi::split_timestamp(&line.text);
        let styled = if line.error {
            Decoder::default().record(body)
        } else {
            self.decoders
                .entry(line.container.clone())
                .or_default()
                .record(body)
        };
        let prefix = format!("[{} / {}] ", line.service, line.name);
        line.text = styled.text.clone();
        *self.services.entry(line.service.clone()).or_default() += 1;
        self.lines.push_back(Record {
            line,
            styled,
            prefix,
        });
        while self.lines.len() > LogBuffer::MAX_LINES {
            if let Some(line) = self.lines.pop_front()
                && let Some(count) = self.services.get_mut(&line.service)
            {
                *count -= 1;
                if *count == 0 {
                    self.services.remove(&line.service);
                }
            }
        }
    }
    pub fn services(&self) -> impl Iterator<Item = &String> {
        self.services.keys()
    }
    pub fn retain_sources<'a>(&mut self, active: impl Iterator<Item = &'a str>) {
        let retained = self
            .lines
            .iter()
            .map(|line| line.container.clone())
            .chain(active.map(str::to_owned))
            .collect::<std::collections::HashSet<_>>();
        self.decoders.retain(|id, _| retained.contains(id.as_str()));
    }
    pub fn text(&self, service: Option<&str>) -> String {
        self.lines
            .iter()
            .filter(|line| service.is_none_or(|service| line.service == service))
            .map(|line| line.display())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Docker's since filter is inclusive and second-precision. Keep the exact
/// timestamp plus occurrence counts so retries don't replay old lines, while
/// preserving legitimate identical lines emitted at the same timestamp.
#[derive(Default, Clone)]
struct Cursor {
    latest: Option<DateTime<FixedOffset>>,
    counts: HashMap<String, usize>,
    replayed: HashMap<String, usize>,
    resume: Option<DateTime<FixedOffset>>,
    resume_counts: HashMap<String, usize>,
}
impl Cursor {
    fn restart(&mut self) {
        self.resume = self.latest;
        self.resume_counts = self.counts.clone();
        self.replayed.clear();
    }
    fn since(&self) -> i32 {
        self.latest
            .and_then(|stamp| i32::try_from(stamp.timestamp()).ok())
            .unwrap_or_default()
    }
    fn accept(&mut self, line: &str) -> bool {
        let Some(stamp) = line
            .split_whitespace()
            .next()
            .and_then(|stamp| DateTime::parse_from_rfc3339(stamp).ok())
        else {
            return true;
        };
        if let Some(resume) = self.resume {
            if stamp < resume {
                return false;
            }
            if stamp == resume {
                let replayed = self.replayed.entry(line.into()).or_default();
                *replayed += 1;
                if *replayed <= *self.resume_counts.get(line).unwrap_or(&0) {
                    return false;
                }
            } else {
                self.resume = None;
                self.resume_counts.clear();
                self.replayed.clear();
            }
        }
        if self.latest.is_none_or(|latest| stamp > latest) {
            self.latest = Some(stamp);
            self.counts.clear();
        }
        if self.latest == Some(stamp) {
            *self.counts.entry(line.into()).or_default() += 1;
        }
        true
    }
}

struct Source {
    state: String,
    active: bool,
    cancel: CancellationToken,
    cursor: Cursor,
    retry_after: time::Instant,
}

pub async fn follow(
    docker: Docker,
    project: String,
    generation: u64,
    mut snapshots: watch::Receiver<Vec<Container>>,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
) {
    let _stop = cancel.clone().drop_guard();
    let (log_lines, received_lines) = mpsc::channel(256);
    tokio::spawn(batch_lines(
        generation,
        received_lines,
        events.clone(),
        cancel.clone(),
    ));
    let mut last_status = String::new();
    let mut sources = BTreeMap::<String, Source>::new();
    let mut tasks = FuturesUnordered::new();
    let mut retry = time::interval(Duration::from_secs(2));
    loop {
        let members = snapshots
            .borrow()
            .iter()
            .filter(|container| container.project.as_deref() == Some(&project))
            .cloned()
            .collect::<Vec<_>>();
        sources.retain(|id, source| {
            let present = members.iter().any(|container| container.id == *id);
            if !present {
                source.cancel.cancel();
            }
            present
        });
        for container in &members {
            if let Some(source) = sources.get(&container.id) {
                if source.active {
                    if source.state != container.state {
                        source.cancel.cancel();
                    }
                    continue;
                }
                if source.state == container.state && !container.running() {
                    continue;
                }
                if source.state == container.state && time::Instant::now() < source.retry_after {
                    continue;
                }
            }
            let mut cursor = sources
                .get(&container.id)
                .map(|source| source.cursor.clone())
                .unwrap_or_default();
            cursor.restart();
            let token = cancel.child_token();
            sources.insert(
                container.id.clone(),
                Source {
                    state: container.state.clone(),
                    active: true,
                    cancel: token.clone(),
                    cursor: cursor.clone(),
                    retry_after: time::Instant::now(),
                },
            );
            tasks.push(tokio::spawn(stream(
                docker.clone(),
                container.clone(),
                generation,
                log_lines.clone(),
                token,
                cursor,
            )));
        }
        let active = members
            .iter()
            .filter(|container| {
                container.running()
                    && sources
                        .get(&container.id)
                        .is_some_and(|source| source.active)
            })
            .count();
        let status = format!(
            "{} project containers · {active} live streams · up to 1,500 lines · merged as received",
            members.len()
        );
        if status != last_status {
            last_status = status.clone();
            if events
                .send(Event::ProjectLogsStatus { generation, status })
                .await
                .is_err()
            {
                return;
            }
        }
        tokio::select! {
            _ = cancel.cancelled() => return,
            changed = snapshots.changed() => if changed.is_err() { return; },
            _ = retry.tick() => {},
            Some(result) = tasks.next(), if !tasks.is_empty() => {
                if let Ok((id, cursor)) = result && let Some(source) = sources.get_mut(&id) {
                    source.active = false;
                    source.cursor = cursor;
                    source.retry_after = time::Instant::now() + Duration::from_secs(2);
                }
            }
        }
    }
}

pub const MAX_BATCH_LINES: usize = 256;
const MAX_BATCH_BYTES: usize = 64 * 1024;

async fn batch_lines(
    generation: u64,
    mut input: mpsc::Receiver<Line>,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
) {
    let mut pending: Option<Line> = None;
    loop {
        let first = match pending.take() {
            Some(line) => line,
            None => {
                tokio::select! { _ = cancel.cancelled() => return, line = input.recv() => match line { Some(line) => line, None => return } }
            }
        };
        let size = |line: &Line| {
            line.text.len() + line.container.len() + line.service.len() + line.name.len()
        };
        let mut bytes = size(&first);
        let mut lines = vec![first];
        let deadline = time::sleep(Duration::from_millis(50));
        tokio::pin!(deadline);
        while lines.len() < MAX_BATCH_LINES && bytes < MAX_BATCH_BYTES {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = &mut deadline => break,
                line = input.recv() => match line { Some(line) => {
                    let next_bytes = size(&line);
                    if bytes + next_bytes > MAX_BATCH_BYTES { pending = Some(line); break; }
                    bytes += next_bytes; lines.push(line);
                }, None => break },
            }
        }
        tokio::select! { _ = cancel.cancelled() => return, sent = events.send(Event::ProjectLogBatch { generation, lines }) => if sent.is_err() { return; } }
    }
}

async fn stream(
    docker: Docker,
    container: Container,
    generation: u64,
    events: mpsc::Sender<Line>,
    cancel: CancellationToken,
    mut cursor: Cursor,
) -> (String, Cursor) {
    let options = LogsOptionsBuilder::default()
        .follow(container.running())
        .stdout(true)
        .stderr(true)
        .timestamps(true)
        .since(cursor.since())
        .tail(if cursor.latest.is_some() {
            "all"
        } else {
            "200"
        })
        .build();
    let mut stream = docker.logs(&container.id, Some(options));
    let mut decoder = LogBuffer::default();
    loop {
        let frame =
            tokio::select! { _ = cancel.cancelled() => break, frame = stream.next() => frame };
        match frame {
            Some(Ok(frame)) => {
                decoder.push(&frame.into_bytes());
                for text in decoder.lines.drain(..) {
                    let previous = cursor.clone();
                    if cursor.accept(&text)
                        && !emit(&container, generation, text, false, &events, &cancel).await
                    {
                        return (container.id, previous);
                    }
                }
            }
            Some(Err(error)) => {
                emit(
                    &container,
                    generation,
                    format!("Logs unavailable: {error}"),
                    true,
                    &events,
                    &cancel,
                )
                .await;
                break;
            }
            None => break,
        }
    }
    if !cancel.is_cancelled() && !decoder.partial.is_empty() {
        let previous = cursor.clone();
        if cursor.accept(&decoder.partial)
            && !emit(
                &container,
                generation,
                decoder.partial,
                false,
                &events,
                &cancel,
            )
            .await
        {
            cursor = previous;
        }
    }
    (container.id, cursor)
}

async fn emit(
    container: &Container,
    _generation: u64,
    text: String,
    error: bool,
    events: &mpsc::Sender<Line>,
    cancel: &CancellationToken,
) -> bool {
    let line = Line {
        container: container.id.clone(),
        service: container
            .service
            .clone()
            .unwrap_or_else(|| "unknown service".into()),
        name: container.name.clone(),
        text,
        error,
    };
    tokio::select! { _ = cancel.cancelled() => false, sent = events.send(line) => sent.is_ok() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retries_skip_replays_without_losing_identical_lines() {
        let line = "2026-10-12T23:30:00.123456789Z same";
        let mut cursor = Cursor::default();
        assert!(cursor.accept(line));
        assert!(cursor.accept(line));
        cursor.restart();
        assert!(!cursor.accept(line));
        assert!(!cursor.accept(line));
        assert!(cursor.accept(line));
        assert!(!cursor.accept("2026-10-12T23:29:59.999999999Z old"));
        assert!(cursor.accept("2026-10-12T23:30:00.223456789Z new"));
        assert!(cursor.accept("2026-10-12T23:30:00.123456789Z late live line"));
    }

    #[test]
    fn ansi_state_is_per_container_and_copy_omits_docker_timestamp() {
        let mut history = History::default();
        let line = |id: &str, text: &str| Line {
            container: id.into(),
            service: "replica".into(),
            name: id.into(),
            text: format!("2026-10-12T23:30:00.000000001Z {text}"),
            error: false,
        };
        history.push(line("a", "\x1b[32mgreen"));
        history.push(line("b", "neutral"));
        history.push(line("a", "still green\x1b[0m reset"));
        assert_eq!(
            history.lines[0].styled.spans[0].1.foreground,
            Some(crate::ansi::PALETTE[2])
        );
        assert!(history.lines[1].styled.spans.is_empty());
        assert_eq!(history.lines[2].styled.spans[0].0, 0..11);
        let copy = history.text(None);
        assert!(!copy.contains('\x1b'));
        assert!(copy.contains("[replica / a] green"));
        assert!(!copy.contains("2026-10-12T23:30:00.000000001Z"));
        history.push(Line {
            error: true,
            ..line("a", "Logs unavailable")
        });
        assert!(history.lines[3].styled.spans.is_empty());
        history.push(line("b", "2026-10-12T23:30:00Z application date"));
        assert!(
            history
                .text(None)
                .contains("[replica / b] 2026-10-12T23:30:00Z application date")
        );
        for _ in 0..1500 {
            history.push(line("b", "retained"));
        }
        history.retain_sources(["a", "b"].into_iter());
        assert!(
            history.decoders.contains_key("a"),
            "Quiet active sources keep their styles"
        );
        history.retain_sources(["b"].into_iter());
        assert!(
            !history.decoders.contains_key("a"),
            "Removed and expired sources release their decoder"
        );
    }
    #[test]
    fn mixed_service_history_is_bounded_and_filters_copy_text() {
        let mut history = History::default();
        for i in 0..2000 {
            history.push(Line {
                container: "id".into(),
                service: if i % 2 == 0 { "web" } else { "worker" }.into(),
                name: "replica-1".into(),
                text: i.to_string(),
                error: false,
            });
        }
        assert_eq!(history.lines.len(), LogBuffer::MAX_LINES);
        let text = history.text(Some("web"));
        assert!(text.contains("[web / replica-1]"));
        assert!(!text.contains("worker"));
        assert_eq!(history.services().count(), 2);
        for _ in 0..LogBuffer::MAX_LINES {
            history.push(Line {
                container: "id".into(),
                service: "new".into(),
                name: "replica-1".into(),
                text: "replacement".into(),
                error: false,
            });
        }
        assert_eq!(history.services().cloned().collect::<Vec<_>>(), vec!["new"]);
    }

    #[tokio::test]
    async fn batches_preserve_order_and_bound_payloads() {
        let (sender, receiver) = mpsc::channel(256);
        let (events, mut output) = mpsc::channel(16);
        let task = tokio::spawn(batch_lines(9, receiver, events, CancellationToken::new()));
        tokio::spawn(async move {
            for index in 0..1000 {
                sender
                    .send(Line {
                        container: "id".into(),
                        service: "probe".into(),
                        name: "replica".into(),
                        text: format!("{index:04} {}", "x".repeat(500)),
                        error: false,
                    })
                    .await
                    .unwrap();
            }
        });
        let mut texts = Vec::new();
        let mut batches = 0;
        while let Some(Event::ProjectLogBatch { generation, lines }) = output.recv().await {
            assert_eq!(generation, 9);
            assert!(lines.len() <= MAX_BATCH_LINES);
            assert!(
                lines
                    .iter()
                    .map(|line| line.text.len()
                        + line.container.len()
                        + line.service.len()
                        + line.name.len())
                    .sum::<usize>()
                    <= MAX_BATCH_BYTES
            );
            texts.extend(lines.into_iter().map(|line| line.text[..4].to_string()));
            batches += 1;
        }
        task.await.unwrap();
        assert_eq!(
            texts,
            (0..1000)
                .map(|index| format!("{index:04}"))
                .collect::<Vec<_>>()
        );
        assert!(batches < 20);
    }
}
