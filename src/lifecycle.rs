//! Docker lifecycle observation, independent of where an action was requested.
use std::collections::VecDeque;

use bollard::{
    Docker,
    models::{EventMessage, EventMessageTypeEnum},
    query_parameters::EventsOptionsBuilder,
};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::docker::Event;
mod summary;
pub use summary::summarize;

pub const MAX_MARKERS: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    pub container: String,
    pub project: Option<String>,
    pub service: String,
    pub name: String,
    pub timestamp: DateTime<Utc>,
    pub action: String,
    pub message: String,
    pub details: String,
}

fn bounded(value: &str, max: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect()
}

impl Marker {
    pub fn from_event(event: &EventMessage) -> Option<Self> {
        if event.typ != Some(EventMessageTypeEnum::CONTAINER) {
            return None;
        }
        let action = event.action.as_deref()?;
        let description = match action {
            "create" => "Container created",
            "start" => "Container started",
            "stop" => "Container stopped",
            "restart" => "Container restarted",
            "die" => "Container process ended",
            "kill" => "Control signal sent to container",
            "oom" => "Container ran out of memory",
            "pause" => "Container paused",
            "unpause" => "Container resumed",
            "destroy" => "Container removed",
            _ => return None,
        };
        let actor = event.actor.as_ref()?;
        let container = bounded(actor.id.as_deref()?, 128);
        if container.is_empty() {
            return None;
        }
        let attributes = actor.attributes.as_ref();
        let attribute = |key: &str| attributes.and_then(|a| a.get(key)).map(String::as_str);
        let timestamp = event
            .time_nano
            .map(DateTime::from_timestamp_nanos)
            .or_else(|| DateTime::from_timestamp(event.time?, 0))?;
        let mut message = description.to_owned();
        let mut details = format!("Docker event: {action}");
        if let Some(code) = attribute("exitCode").filter(|_| action == "die") {
            let code = bounded(code, 32);
            details.push_str(&format!(" · exit code: {code}"));
            message = match code.parse::<u32>() {
                Ok(0) => "Container finished successfully",
                Ok(1..=127) => "Container exited with an error",
                // Codes such as 137/143 are common during normal restarts.
                // They alone do not establish a crash, OOM, or forced kill.
                _ => "Container process ended",
            }
            .into();
        }
        if let Some(signal) = attribute("signal").filter(|_| action == "kill") {
            let signal = bounded(signal, 32);
            details.push_str(&format!(" · signal: {signal}"));
            let signal = signal.trim().to_ascii_uppercase();
            message = match signal.strip_prefix("SIG").unwrap_or(&signal) {
                "2" | "INT" | "3" | "QUIT" | "15" | "TERM" => "Container shutdown requested",
                "9" | "KILL" => "Container force stop requested",
                _ => "Control signal sent to container",
            }
            .into();
        }
        let name = bounded(attribute("name").unwrap_or(&container), 256);
        Some(Self {
            container,
            timestamp,
            action: action.into(),
            message,
            details,
            project: attribute("com.docker.compose.project").map(|s| bounded(s, 256)),
            service: bounded(
                attribute("com.docker.compose.service").unwrap_or(&name),
                256,
            ),
            name,
        })
    }

    pub fn text(&self) -> String {
        format!(
            "[Docker] {} {}",
            self.timestamp
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            self.message
        )
    }

    pub fn styled(&self) -> crate::ansi::StyledLine {
        crate::ansi::StyledLine {
            text: self.text(),
            timestamp: Some(self.timestamp),
            details: Some(self.details.clone()),
            annotation: true,
            ..Default::default()
        }
    }

    pub fn project_line(&self) -> crate::project_logs::Line {
        crate::project_logs::Line {
            container: self.container.clone(),
            service: self.service.clone(),
            name: self.name.clone(),
            text: format!("{} {}", self.timestamp.to_rfc3339(), self.text()),
            error: false,
        }
    }
}

/// Small replay window. Docker's since parameter replays the boundary second.
#[derive(Default)]
struct Cursor {
    latest: Option<i64>,
    seen: VecDeque<(i64, String, String, Option<EventMessageTypeEnum>)>,
}
impl Cursor {
    fn accept(&mut self, event: &EventMessage, replay_floor: Option<i64>) -> bool {
        let Some(time) = event
            .time_nano
            .or_else(|| event.time?.checked_mul(1_000_000_000))
        else {
            return true;
        };
        if replay_floor.is_some_and(|floor| time < floor) {
            return false;
        }
        let key = (
            time,
            bounded(
                event
                    .actor
                    .as_ref()
                    .and_then(|a| a.id.as_deref())
                    .unwrap_or(""),
                128,
            ),
            bounded(event.action.as_deref().unwrap_or(""), 128),
            event.typ,
        );
        if self.seen.contains(&key) {
            return false;
        }
        self.latest = Some(self.latest.map_or(time, |old| old.max(time)));
        self.seen.push_back(key);
        while self.seen.len() > MAX_MARKERS {
            self.seen.pop_front();
        }
        true
    }
}

pub async fn follow(
    docker: Docker,
    session: u64,
    changed: watch::Sender<u64>,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
) {
    let mut cursor = Cursor::default();
    // Include startup races between establishing the connection and polling /events.
    let started = Utc::now().timestamp();
    loop {
        let replay_floor = cursor.latest;
        let since = cursor
            .latest
            .map_or(started, |time| time / 1_000_000_000)
            .to_string();
        let options = EventsOptionsBuilder::default().since(&since).build();
        let mut stream = docker.events(Some(options));
        let mut live = false;
        loop {
            let event = tokio::select! {
                _ = cancel.cancelled() => return,
                event = stream.next() => event,
            };
            let Some(Ok(event)) = event else {
                break;
            };
            if !live {
                live = true;
                if !send(
                    &events,
                    Event::LifecycleStatus {
                        session,
                        live: true,
                    },
                    &cancel,
                )
                .await
                {
                    return;
                }
            }
            if !cursor.accept(&event, replay_floor) {
                continue;
            }
            // Keep inventory refreshes for every object type, including networks/images.
            changed.send_modify(|value| *value = value.wrapping_add(1));
            if let Some(marker) = Marker::from_event(&event)
                && !send(&events, Event::Lifecycle { session, marker }, &cancel).await
            {
                return;
            }
        }
        if !send(
            &events,
            Event::LifecycleStatus {
                session,
                live: false,
            },
            &cancel,
        )
        .await
        {
            return;
        }
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {},
        }
    }
}

async fn send(events: &mpsc::Sender<Event>, event: Event, cancel: &CancellationToken) -> bool {
    tokio::select! {
        _ = cancel.cancelled() => false,
        result = events.send(event) => result.is_ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(action: &str, time: i64) -> EventMessage {
        serde_json::from_value(serde_json::json!({"Type":"container","Action":action,"timeNano":time,
            "Actor":{"ID":"fixture","Attributes":{"name":"probe-1","com.docker.compose.project":"probe","com.docker.compose.service":"web","exitCode":"137","signal":"15"}}})).unwrap()
    }
    #[test]
    fn lifecycle_labels_metadata_and_bounds() {
        let marker = Marker::from_event(&event("die", 1_000_000_001)).unwrap();
        assert_eq!(marker.message, "Container process ended");
        assert!(marker.details.contains("exit code: 137"));
        assert_eq!(marker.project.as_deref(), Some("probe"));
        assert_eq!(marker.service, "web");
        assert_eq!(marker.timestamp.timestamp_subsec_nanos(), 1);
        assert!(marker.text().starts_with("[Docker] 1970-01-01T00:00:01Z"));
        assert!(Marker::from_event(&event("exec_start", 1)).is_none());
        let mut network = event("start", 1);
        network.typ = Some(EventMessageTypeEnum::NETWORK);
        assert!(Marker::from_event(&network).is_none());
        let mut huge = event("kill", 1);
        huge.actor
            .as_mut()
            .unwrap()
            .attributes
            .as_mut()
            .unwrap()
            .insert("name".into(), "\x1b\n界".repeat(1000));
        let marker = Marker::from_event(&huge).unwrap();
        assert_eq!(marker.name.chars().count(), 256);
        assert!(!marker.name.contains('\n'));
        assert_eq!(marker.message, "Container shutdown requested");
        assert!(marker.details.contains("signal: 15"));
    }

    #[test]
    fn friendly_signal_and_exit_messages_preserve_diagnostics_without_guessing_cause() {
        for (signal, expected) in [
            ("3", "Container shutdown requested"),
            ("SIGQUIT", "Container shutdown requested"),
            ("15", "Container shutdown requested"),
            ("TERM", "Container shutdown requested"),
            ("2", "Container shutdown requested"),
            ("9", "Container force stop requested"),
            ("SIGKILL", "Container force stop requested"),
            ("10", "Control signal sent to container"),
        ] {
            let mut e = event("kill", 1);
            e.actor
                .as_mut()
                .unwrap()
                .attributes
                .as_mut()
                .unwrap()
                .insert("signal".into(), signal.into());
            let marker = Marker::from_event(&e).unwrap();
            assert_eq!(marker.message, expected);
            assert!(marker.styled().details.unwrap().contains(signal));
            assert!(
                !marker.text().contains(&format!("signal {signal}")),
                "Technical signal values belong in details"
            );
        }
        for (code, expected) in [
            ("0", "Container finished successfully"),
            ("1", "Container exited with an error"),
            ("137", "Container process ended"),
            ("143", "Container process ended"),
            ("unknown", "Container process ended"),
        ] {
            let mut e = event("die", 1);
            e.actor
                .as_mut()
                .unwrap()
                .attributes
                .as_mut()
                .unwrap()
                .insert("exitCode".into(), code.into());
            let marker = Marker::from_event(&e).unwrap();
            assert_eq!(marker.message, expected);
            assert!(marker.details.contains(code));
            assert!(!marker.text().contains("exit code"));
        }
    }
    #[test]
    fn reconnect_replay_keeps_new_equal_time_events_and_bounds_memory() {
        let mut cursor = Cursor::default();
        assert!(cursor.accept(&event("stop", 1_000_000_001), None));
        let floor = cursor.latest;
        assert!(!cursor.accept(&event("stop", 1_000_000_001), floor));
        assert!(!cursor.accept(&event("die", 1_000_000_000), floor));
        assert!(cursor.accept(&event("start", 1_000_000_001), floor));
        for i in 0..1000 {
            assert!(cursor.accept(&event("restart", 2_000_000_000 + i), None));
        }
        assert_eq!(cursor.seen.len(), MAX_MARKERS);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn interrupted_stream_reconnects_replays_once_and_cancels() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let docker =
            Docker::connect_with_unix(path.to_str().unwrap(), 10, bollard::API_DEFAULT_VERSION)
                .unwrap();
        let fixture = tokio::spawn(async move {
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(stream.read_u8().await.unwrap());
                    assert!(request.len() < 8192);
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.contains("/events?since="));
                if attempt == 1 {
                    assert!(request.contains("since=1 "));
                }
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
                let mut records = vec![event("stop", 1_000_000_001)];
                if attempt == 1 {
                    records.push(event("start", 1_000_000_002));
                }
                let body = records
                    .iter()
                    .map(|e| format!("{}\n", serde_json::to_string(e).unwrap()))
                    .collect::<String>();
                stream
                    .write_all(format!("{:x}\r\n{}\r\n", body.len(), body).as_bytes())
                    .await
                    .unwrap();
                if attempt == 0 {
                    // An abrupt EOF, without the terminating HTTP chunk.
                    drop(stream);
                } else {
                    let mut byte = [0];
                    let _ = stream.read(&mut byte).await;
                }
            }
        });
        let (events, mut received) = mpsc::channel(16);
        let (changed, _) = watch::channel(0);
        let cancel = CancellationToken::new();
        let follower = tokio::spawn(follow(docker, 9, changed, events, cancel.clone()));
        let mut markers = Vec::new();
        let mut interrupted = false;
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while let Some(event) = received.recv().await {
                match event {
                    Event::Lifecycle { session: 9, marker } => {
                        markers.push(marker.action);
                        if markers.len() == 2 {
                            break;
                        }
                    }
                    Event::LifecycleStatus {
                        session: 9,
                        live: false,
                    } => interrupted = true,
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        cancel.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(2), follower)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), fixture)
            .await
            .unwrap()
            .unwrap();
        assert!(interrupted);
        assert_eq!(markers, ["stop", "start"]);
    }
}
