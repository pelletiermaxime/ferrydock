//! Resource summaries remain visible across the selected container's tabs.
use super::{FerryDock, ui::palette};
use crate::{
    metrics::{self, Sample},
    model::Container,
};
use gpui::{prelude::*, *};
use gpui_kit::{base::TestSupportExt, component::chart::AreaChart};
use std::{
    collections::{HashMap, VecDeque},
    time::Instant,
};

#[derive(Default)]
pub(super) struct Metrics {
    pub(super) session: Option<u64>,
    pub(super) containers: HashMap<String, History>,
}

#[derive(Default)]
pub(super) struct History {
    pub(super) samples: VecDeque<Sample>,
    pub(super) error: Option<String>,
    stale: bool,
}

impl History {
    pub(super) fn push(&mut self, result: Result<Sample, String>) {
        match result {
            Ok(sample) => {
                if sample.reset {
                    self.samples.clear();
                }
                self.samples.push_back(sample);
                while self.samples.len() > metrics::HISTORY_SAMPLES {
                    self.samples.pop_front();
                }
                self.error = None;
                self.stale = false;
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub(super) fn live(&self) -> Option<&Sample> {
        self.samples.back().filter(|sample| {
            self.error.is_none() && sample.collected.elapsed() < metrics::STALE_AFTER
        })
    }
}

#[derive(Clone, Debug)]
struct ChartPoint {
    collected: Instant,
    label: SharedString,
    value: f64,
    display: SharedString,
}

fn time_label(age: std::time::Duration) -> String {
    let seconds = age.as_secs();
    if seconds < 2 {
        "Now".into()
    } else if seconds < 60 {
        format!("{seconds}s ago")
    } else {
        format!("{}:{:02} ago", seconds / 60, seconds % 60)
    }
}

// A zero baseline with modest headroom, rather than a fixed capacity scale
// that flattens low memory usage. Tick labels always expose the chosen scale.
fn chart_upper(peak: f64) -> f64 {
    let target = (peak * 1.15).max(0.01);
    let magnitude = 10.0_f64.powf(target.log10().floor());
    [1.0, 1.25, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0]
        .into_iter()
        .map(|step| step * magnitude)
        .find(|upper| *upper >= target)
        .unwrap_or(magnitude * 10.0)
}

fn axis_value(value: f64, upper: f64, cpu: bool) -> String {
    let decimals = if (cpu && upper < 0.1) || (!cpu && upper < 1.0) {
        3
    } else {
        2
    };
    let number = format!("{value:.decimals$}");
    let number = number.trim_end_matches('0').trim_end_matches('.');
    if cpu {
        format!("{number}%")
    } else {
        format!("{number} MiB")
    }
}

impl History {
    fn chart_points(&self, cpu: bool, now: Instant) -> Vec<ChartPoint> {
        let mut points = Vec::<ChartPoint>::new();
        for sample in &self.samples {
            let age = now.saturating_duration_since(sample.collected);
            if age > metrics::HISTORY_WINDOW {
                continue;
            }
            let value = if cpu {
                sample.cpu_percent
            } else {
                sample
                    .memory_used
                    .map(|bytes| bytes as f64 / (1024.0 * 1024.0))
            };
            let Some(value) = value.filter(|value| value.is_finite()) else {
                // Start a new series after missing readings; do not smooth a gap
                // into data that Docker never measured.
                points.clear();
                continue;
            };
            if let Some(previous) = points.last() {
                if sample.collected <= previous.collected {
                    continue;
                }
                if sample.collected.duration_since(previous.collected) > metrics::STALE_AFTER {
                    points.clear();
                }
            }
            points.push(ChartPoint {
                collected: sample.collected,
                label: time_label(age).into(),
                value,
                display: if cpu {
                    format!("{value:.2}%").into()
                } else {
                    metrics::bytes(sample.memory_used.unwrap()).into()
                },
            });
        }
        points
    }
}

impl Metrics {
    pub(super) fn expire(&mut self) -> bool {
        let mut changed = false;
        for history in self.containers.values_mut() {
            let stale = history
                .samples
                .back()
                .is_some_and(|sample| sample.collected.elapsed() >= metrics::STALE_AFTER);
            if stale != history.stale {
                history.stale = stale;
                changed = true;
            }
        }
        changed
    }
}

impl FerryDock {
    fn metrics_status(&self, container: &Container) -> &'static str {
        if !container.running() {
            return "Stopped";
        }
        if self.connection.is_none() {
            return "Disconnected";
        }
        match self.metrics.containers.get(&container.id) {
            Some(history) if history.error.is_some() => "Unavailable",
            Some(history) if history.live().is_some() => "Live",
            Some(history) if !history.samples.is_empty() => "Stale",
            _ => "Collecting…",
        }
    }

    fn live_metrics(&self, container: &Container) -> Option<&Sample> {
        if !container.running() || self.connection.is_none() {
            return None;
        }
        self.metrics.containers.get(&container.id)?.live()
    }

    pub(super) fn metrics_row(&self, container: &Container, cx: &App) -> impl IntoElement {
        let colors = palette(cx);
        let sample = self.live_metrics(container);
        let cpu = sample
            .and_then(|sample| sample.cpu_percent)
            .map(|value| format!("{value:.1}%"))
            .unwrap_or_else(|| "—".into());
        let memory = sample
            .and_then(|sample| sample.memory_used)
            .map(|used| {
                let limit = sample
                    .and_then(|sample| sample.limits.as_ref())
                    .map(|limits| {
                        limits
                            .memory
                            .map(metrics::bytes)
                            .unwrap_or_else(|| "No limit".into())
                    })
                    .unwrap_or_else(|| "Limit unknown".into());
                format!("{} / {limit}", metrics::bytes(used))
            })
            .unwrap_or_else(|| "—".into());
        let status = self.metrics_status(container);
        div()
            .id(SharedString::from(format!("metrics-row-{}", container.id)))
            .test_support()
            .debug_selector({
                let id = container.id.clone();
                move || format!("metrics-row-{id}")
            })
            .w(px(230.0))
            .flex_shrink_0()
            .text_size(px(11.0))
            .text_color(rgb(colors.muted))
            .child(div().text_ellipsis().child(format!(
                "CPU {cpu}{}",
                if status == "Live" {
                    String::new()
                } else {
                    format!(" · {status}")
                }
            )))
            .child(div().text_ellipsis().child(format!("Memory {memory}")))
    }

    pub(super) fn metrics_summary(&self, container: &Container, cx: &App) -> impl IntoElement {
        let colors = palette(cx);
        let sample = self.live_metrics(container);
        let status = self.metrics_status(container);
        let cpu = sample
            .and_then(|sample| sample.cpu_percent)
            .map(|value| format!("{value:.1}%"))
            .unwrap_or_else(|| "—".into());
        let cpu_limit = sample
            .and_then(|sample| sample.limits.as_ref())
            .map(|limits| {
                let limit = limits
                    .cpu_cores
                    .map(|cores| format!("Limit {cores:.2} CPUs"))
                    .unwrap_or_else(|| "No CPU quota".into());
                match &limits.cpuset {
                    Some(set) => format!("{limit} · cores {set}"),
                    None => limit,
                }
            })
            .unwrap_or_else(|| "CPU limit unavailable".into());
        let cpu_limit = format!("{cpu_limit} · 100% = 1 core");
        let memory = sample
            .and_then(|sample| sample.memory_used)
            .map(metrics::bytes)
            .unwrap_or_else(|| "—".into());
        let memory_percent = sample
            .and_then(Sample::memory_percent)
            .map(|value| format!("{value:.1}%"))
            .unwrap_or_else(|| "—".into());
        let memory_limit = match sample {
            Some(sample) => match &sample.limits {
                Some(limits) => match limits.memory {
                    Some(limit) => format!("Limit {} · {memory_percent}", metrics::bytes(limit),),
                    None => sample
                        .memory_capacity
                        .map(|capacity| {
                            format!(
                                "No container limit · {memory_percent} of {} Docker capacity",
                                metrics::bytes(capacity)
                            )
                        })
                        .unwrap_or_else(|| "No container limit · capacity unavailable".into()),
                },
                None => "Memory limit unavailable".into(),
            },
            None => format!("No live memory data · {status}"),
        };
        let card =
            |id: &'static str, title: &str, value: String, caption: String, cpu_chart: bool| {
                div()
                    .id(id)
                    .test_support()
                    .debug_selector(move || id.to_string())
                    .flex_1()
                    .min_w_0()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(colors.panel))
                    .border_1()
                    .border_color(rgb(colors.border))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(colors.muted))
                                    .child(title.to_owned()),
                            )
                            .child(
                                div()
                                    .text_size(px(18.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(value),
                            ),
                    )
                    .child(self.metrics_chart(container, cpu_chart, cx))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(colors.muted))
                            .text_ellipsis()
                            .child(caption),
                    )
            };
        div()
            .id("container-metrics")
            .test_support()
            .debug_selector(|| "container-metrics".into())
            .flex_shrink_0()
            .px_4()
            .pb_2()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_size(px(10.0))
                    .text_color(rgb(colors.muted))
                    .pb_1()
                    .child(format!("Resources · {status}"))
                    .child("Recent history · hover for values"),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(card("cpu-metrics", "CPU", cpu, cpu_limit, true))
                    .child(card(
                        "memory-metrics",
                        "Memory",
                        memory,
                        memory_limit,
                        false,
                    )),
            )
    }

    fn metrics_chart(&self, container: &Container, cpu: bool, cx: &App) -> impl IntoElement {
        let colors = palette(cx);
        let points = self
            .metrics
            .containers
            .get(&container.id)
            .map(|history| history.chart_points(cpu, Instant::now()))
            .unwrap_or_default();
        let live = self.live_metrics(container).is_some();
        let color: Hsla = rgb(if live { colors.accent } else { colors.muted }).into();
        let upper = chart_upper(points.iter().map(|point| point.value).fold(0.0, f64::max));
        let chart_id = SharedString::from(format!(
            "{}-{}",
            if cpu { "cpu-plot" } else { "memory-plot" },
            container.id
        ));
        let mut chart = div()
            .id(if cpu { "cpu-history" } else { "memory-history" })
            .test_support()
            .debug_selector(move || {
                if cpu {
                    "cpu-history".into()
                } else {
                    "memory-history".into()
                }
            })
            .h(px(96.0))
            .w_full()
            .my_2();
        if points.len() < 2 {
            return chart
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.0))
                .text_color(rgb(colors.muted))
                .child(if points.is_empty() {
                    "No history available"
                } else {
                    "Collecting history…"
                });
        }
        chart = chart.child(
            AreaChart::new(points)
                .id(chart_id)
                .x(|point: &ChartPoint| point.label.clone())
                .y(|point: &ChartPoint| point.value)
                .stroke(color)
                .fill(color.opacity(0.14))
                .natural()
                .appear(false)
                .y_domain(0.0, upper)
                .y_padding(0.0, 0.0)
                .y_axis(true)
                .y_tick_count(3)
                .y_tick_format(move |value| axis_value(value, upper, cpu))
                .x_tick_count(3)
                .grid_dashed(true)
                .tooltip_content(move |point: &ChartPoint, _, cx| {
                    let colors = palette(cx);
                    div()
                        .id(if cpu {
                            "cpu-hover-value"
                        } else {
                            "memory-hover-value"
                        })
                        .role(Role::Tooltip)
                        .aria_label(format!(
                            "{}: {}",
                            if cpu { "CPU" } else { "Memory" },
                            point.display
                        ))
                        .test_support()
                        .debug_selector(move || {
                            if cpu {
                                "cpu-hover-value".into()
                            } else {
                                "memory-hover-value".into()
                            }
                        })
                        .flex()
                        .flex_col()
                        .gap_1()
                        .text_size(px(12.0))
                        .text_color(rgb(colors.text))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(colors.muted))
                                .child(format!(
                                    "{} · {}",
                                    if cpu { "CPU" } else { "Memory" },
                                    time_label(point.collected.elapsed())
                                )),
                        )
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child(point.display.clone()),
                        )
                }),
        );
        chart
    }
}

#[cfg(test)]
mod tests {
    use super::{History, Metrics, axis_value, chart_upper};
    use crate::metrics::{self, Sample};
    use std::time::Instant;
    #[test]
    fn chart_uses_memory_bytes_and_starts_a_new_series_after_gaps() {
        let now = Instant::now();
        let mut history = History::default();
        for age in [30, 28, 8, 6] {
            history.push(Ok(Sample {
                collected: now - std::time::Duration::from_secs(age),
                cpu_percent: Some(0.25),
                memory_used: Some(25 * 1024 * 1024),
                memory_capacity: Some(2 * 1024 * 1024 * 1024),
                limits: Some(metrics::Limits {
                    memory: Some(2 * 1024 * 1024 * 1024),
                    ..Default::default()
                }),
                reset: false,
            }));
        }
        let points = history.chart_points(false, now);
        assert_eq!(
            points.len(),
            2,
            "Collection gaps must not be smoothed into measured data"
        );
        assert_eq!(points[0].value, 25.0);
        assert_eq!(points[0].display.as_ref(), "25.0 MiB");
        assert_eq!(
            chart_upper(points[0].value),
            30.0,
            "The chart scales to usage rather than the 2 GiB limit"
        );
        history.samples.back_mut().unwrap().cpu_percent = None;
        assert!(history.chart_points(true, now).is_empty());
        assert_eq!(history.chart_points(false, now).len(), 2);
        assert!(chart_upper(0.0) > 0.0);
        assert_eq!(axis_value(0.005, 0.01, true), "0.005%");
        assert_eq!(axis_value(250.0, 250.0, false), "250 MiB");
    }
    #[test]
    fn stale_failure_recovery_and_bounded_history() {
        let sample = || Sample {
            collected: Instant::now(),
            cpu_percent: Some(0.0),
            memory_used: Some(1),
            memory_capacity: Some(100),
            limits: None,
            reset: false,
        };
        let mut history = History::default();
        history.push(Ok(sample()));
        assert!(history.live().is_some());
        history.push(Err("not available".into()));
        assert!(history.live().is_none());
        for _ in 0..100 {
            history.push(Ok(sample()));
        }
        assert_eq!(history.samples.len(), metrics::HISTORY_SAMPLES);
        assert!(history.live().is_some());
        history.samples.back_mut().unwrap().collected = Instant::now() - metrics::STALE_AFTER;
        assert!(history.live().is_none());
        let mut state = Metrics::default();
        state.containers.insert("fixture".into(), history);
        assert!(state.expire());
        assert!(!state.expire());
        let history = state.containers.get_mut("fixture").unwrap();
        let mut reset = sample();
        reset.reset = true;
        history.push(Ok(reset));
        assert_eq!(history.samples.len(), 1);
    }
}
