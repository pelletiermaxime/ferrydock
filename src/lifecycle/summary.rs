//! Project raw lifecycle events into concise stop phases and confirmed restarts.
use super::Marker;
use std::collections::VecDeque;

pub fn summarize(markers: &VecDeque<Marker>) -> Vec<Marker> {
    let mut ordered = markers.iter().cloned().collect::<Vec<_>>();
    ordered.sort_by_key(|marker| marker.timestamp);
    let mut keep = vec![true; ordered.len()];
    for index in 0..ordered.len() {
        if ordered[index].action != "restart" {
            continue;
        }
        let restart = &ordered[index];
        let mut related = Vec::new();
        let mut start = None;
        let mut ended = false;
        for prior in (0..index).rev() {
            let candidate = &ordered[prior];
            if restart
                .timestamp
                .signed_duration_since(candidate.timestamp)
                .num_seconds()
                > 300
            {
                break;
            }
            if candidate.container != restart.container {
                continue;
            }
            if related.len() >= 16 {
                break;
            }
            match candidate.action.as_str() {
                "start" if start.is_none() => {
                    if restart
                        .timestamp
                        .signed_duration_since(candidate.timestamp)
                        .num_seconds()
                        > 10
                    {
                        break;
                    }
                    start = Some(candidate.timestamp);
                }
                "die" | "stop" => {
                    let Some(started) = start else {
                        break;
                    };
                    // A long-stopped container may itself be restarted later.
                    // Keep that earlier, independent stop visible.
                    if !ended
                        && started
                            .signed_duration_since(candidate.timestamp)
                            .num_seconds()
                            > 10
                    {
                        break;
                    }
                    ended = true;
                }
                "kill"
                    if ended
                        && matches!(
                            candidate.message.as_str(),
                            "Container shutdown requested" | "Container force stop requested"
                        ) => {}
                _ => break,
            }
            related.push(prior);
        }
        if start.is_none() || !ended {
            continue;
        }
        related.reverse();
        let details = related
            .iter()
            .copied()
            .chain(std::iter::once(index))
            .map(|i| {
                let marker = &ordered[i];
                format!(
                    "{}  {}",
                    marker
                        .timestamp
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    marker.details
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        for prior in &related {
            // Preserve genuine process errors as separate annotations.
            if !process_error(&ordered[*prior]) {
                keep[*prior] = false;
            }
        }
        ordered[index].details = format!(
            "Container restarted · {} Docker events\n{details}",
            related.len() + 1
        );
    }
    summarize_stops(&mut ordered, &mut keep);
    ordered
        .into_iter()
        .zip(keep)
        .filter_map(|(marker, keep)| keep.then_some(marker))
        .collect()
}

fn stop_request(marker: &Marker) -> bool {
    marker.action == "kill"
        && matches!(
            marker.message.as_str(),
            "Container shutdown requested" | "Container force stop requested"
        )
}

fn process_error(marker: &Marker) -> bool {
    marker.action == "die" && marker.message == "Container exited with an error"
}

fn details(title: &str, indices: &[usize], markers: &[Marker]) -> String {
    let events = indices
        .iter()
        .map(|index| {
            let marker = &markers[*index];
            format!(
                "{}  {}",
                marker
                    .timestamp
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                marker.details
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{title} · {} Docker events\n{events}", indices.len())
}

fn summarize_stops(markers: &mut [Marker], keep: &mut [bool]) {
    for index in 0..markers.len() {
        if !keep[index] || !stop_request(&markers[index]) {
            continue;
        }
        let mut related = vec![index];
        let mut requests = vec![index];
        let mut completion = None;
        let mut ended_at = None;
        for next in index + 1..markers.len() {
            if markers[next]
                .timestamp
                .signed_duration_since(markers[index].timestamp)
                .num_seconds()
                > 300
            {
                break;
            }
            if markers[next].container != markers[index].container {
                continue;
            }
            if !keep[next] || related.len() >= 16 {
                break;
            }
            if ended_at.is_some_and(|at| {
                markers[next]
                    .timestamp
                    .signed_duration_since(at)
                    .num_seconds()
                    > 10
            }) {
                break;
            }
            match markers[next].action.as_str() {
                "kill" if ended_at.is_none() && stop_request(&markers[next]) => requests.push(next),
                "stop" => {
                    completion = Some(next);
                    ended_at = Some(markers[next].timestamp);
                }
                "die" => {
                    ended_at = Some(markers[next].timestamp);
                    if completion.is_none() && !process_error(&markers[next]) {
                        completion = Some(next);
                    }
                }
                _ => break,
            }
            related.push(next);
        }
        let request_details = details("Container stop requested", &requests, markers);
        let stopped_details = completion.map(|_| details("Container stopped", &related, markers));
        for next in related.iter().copied().skip(1) {
            if !process_error(&markers[next]) {
                keep[next] = false;
            }
        }
        markers[index].message = "Container stop requested".into();
        markers[index].details = request_details;
        if let Some(completion) = completion {
            keep[completion] = true;
            markers[completion].message = "Container stopped".into();
            markers[completion].details = stopped_details.unwrap();
        }
    }
    // Partial event history may contain stop + die without the request. Prefer
    // one explicit stopped marker, while leaving independent process exits alone.
    for index in 0..markers.len() {
        if !keep[index] || markers[index].action != "stop" {
            continue;
        }
        let mut related = vec![index];
        let before = (0..index)
            .rev()
            .find(|next| keep[*next] && markers[*next].container == markers[index].container);
        let after = (index + 1..markers.len())
            .find(|next| keep[*next] && markers[*next].container == markers[index].container);
        for next in [before, after].into_iter().flatten() {
            if markers[next].action == "die"
                && !process_error(&markers[next])
                && markers[next]
                    .timestamp
                    .signed_duration_since(markers[index].timestamp)
                    .num_seconds()
                    .abs()
                    <= 10
            {
                related.push(next);
                keep[next] = false;
            }
        }
        if related.len() > 1 {
            related.sort_unstable();
            markers[index].details = details("Container stopped", &related, markers);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn marker(action: &str, time: i64, signal: &str, code: &str, id: &str) -> Marker {
        Marker::from_event(
            &serde_json::from_value(
                serde_json::json!({"Type":"container","Action":action,"time":time,
            "Actor":{"ID":id,"Attributes":{"name":id,"signal":signal,"exitCode":code}}}),
            )
            .unwrap(),
        )
        .unwrap()
    }
    fn restart_sequence(id: &str) -> VecDeque<Marker> {
        [
            ("kill", 1, "3"),
            ("kill", 11, "9"),
            ("stop", 12, ""),
            ("die", 12, ""),
            ("start", 12, ""),
            ("restart", 12, ""),
        ]
        .into_iter()
        .map(|(action, time, signal)| marker(action, time, signal, "137", id))
        .collect()
    }
    #[test]
    fn six_events_become_one_confirmed_restart_with_all_diagnostics() {
        let raw = restart_sequence("one");
        let result = summarize(&raw);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].message, "Container restarted");
        assert!(result[0].details.contains("6 Docker events"));
        for detail in [
            "signal: 3",
            "signal: 9",
            "exit code: 137",
            "Docker event: stop",
            "Docker event: start",
        ] {
            assert!(result[0].details.contains(detail));
        }
        assert_eq!(raw.len(), 6, "Raw session events stay available");
    }
    #[test]
    fn grouping_requires_confirmation_and_preserves_other_containers_and_errors() {
        let mut raw = restart_sequence("one");
        raw.pop_back();
        assert_eq!(
            summarize(&raw).len(),
            3,
            "An unfinished or separate stop/start is not a confirmed restart"
        );
        raw[3] = marker("die", 12, "", "1", "one");
        raw.push_back(marker("stop", 12, "", "0", "other"));
        raw.push_back(marker("restart", 12, "", "0", "one"));
        let result = summarize(&raw);
        assert_eq!(
            result.len(),
            3,
            "One summary, the process error, and the other container's stop"
        );
        assert!(
            result
                .iter()
                .any(|m| m.message == "Container exited with an error")
        );
        assert!(
            result
                .iter()
                .any(|m| m.container == "other" && m.action == "stop")
        );
        assert!(result.iter().any(|m| m.action == "restart"));
    }
    #[test]
    fn independent_stops_repeated_restarts_and_oom_stay_visible() {
        let mut raw = VecDeque::from([
            marker("stop", 1, "", "0", "one"),
            marker("start", 90, "", "0", "one"),
            marker("restart", 90, "", "0", "one"),
        ]);
        assert_eq!(summarize(&raw).len(), 3);
        raw = restart_sequence("one");
        let mut second = restart_sequence("one");
        for m in &mut second {
            m.timestamp += chrono::Duration::seconds(20);
        }
        raw.extend(second);
        assert_eq!(summarize(&raw).len(), 2);
        raw.push_back(marker("oom", 34, "", "137", "one"));
        raw.push_back(marker("die", 35, "", "137", "one"));
        raw.push_back(marker("start", 36, "", "0", "one"));
        assert!(summarize(&raw).iter().any(|m| m.action == "oom"));
        assert!(summarize(&raw).iter().any(|m| m.action == "die"));
    }

    #[test]
    fn stop_is_one_request_and_one_completion_in_either_exit_order() {
        for reversed in [false, true] {
            let mut raw = VecDeque::from([marker("kill", 1, "3", "", "one")]);
            assert_eq!(summarize(&raw)[0].message, "Container stop requested");
            raw.push_back(marker("kill", 11, "9", "", "one"));
            let pending = summarize(&raw);
            assert_eq!(pending.len(), 1);
            assert!(
                pending[0].details.contains("signal: 3")
                    && pending[0].details.contains("signal: 9")
            );
            for action in if reversed {
                ["die", "stop"]
            } else {
                ["stop", "die"]
            } {
                raw.push_back(marker(action, 12, "", "137", "one"));
                assert_eq!(summarize(&raw).len(), 2);
            }
            let result = summarize(&raw);
            assert_eq!(
                result
                    .iter()
                    .map(|m| m.message.as_str())
                    .collect::<Vec<_>>(),
                ["Container stop requested", "Container stopped"]
            );
            assert!(
                result[1].details.contains("signal: 9")
                    && result[1].details.contains("exit code: 137")
            );
            raw.push_back(marker("start", 24, "", "0", "one"));
            assert_eq!(
                summarize(&raw)
                    .iter()
                    .map(|m| m.message.as_str())
                    .collect::<Vec<_>>(),
                [
                    "Container stop requested",
                    "Container stopped",
                    "Container started"
                ]
            );
        }
    }

    #[test]
    fn stop_grouping_preserves_errors_oom_other_containers_and_partial_history() {
        let raw = VecDeque::from([
            marker("kill", 1, "15", "", "one"),
            marker("die", 2, "", "1", "one"),
            marker("stop", 2, "", "1", "one"),
            marker("stop", 2, "", "0", "other"),
        ]);
        let result = summarize(&raw);
        assert_eq!(result.len(), 4);
        assert!(
            result
                .iter()
                .any(|m| m.message == "Container exited with an error")
        );
        let raw = VecDeque::from([
            marker("kill", 1, "15", "", "one"),
            marker("oom", 2, "", "", "one"),
            marker("die", 3, "", "137", "one"),
        ]);
        assert!(summarize(&raw).iter().any(|m| m.action == "oom"));
        let raw = VecDeque::from([
            marker("stop", 1, "", "0", "one"),
            marker("die", 1, "", "137", "one"),
        ]);
        assert_eq!(summarize(&raw).len(), 1);
        assert!(summarize(&raw)[0].details.contains("exit code: 137"));
        assert_eq!(
            summarize(&VecDeque::from([marker("die", 1, "", "137", "one")]))[0].message,
            "Container process ended"
        );
    }
}
