//! Native layer analysis, using Kit controls and virtualized Tree.
use super::{
    FerryDock,
    ui::{empty_state, palette},
};
use crate::{
    docker::{Command, Event},
    image_analysis::{FileNode, Report},
};
use gpui::{prelude::*, *};
use gpui_kit::{
    base::TestSupportExt,
    component::{
        Sizable,
        alert::Alert,
        list::ListItem,
        resizable::{ResizableState, h_resizable, resizable_panel},
        tab::{Tab, TabBar},
        tree::{Tree, TreeItem, TreeState},
    },
};
use std::{cell::Cell, sync::Arc};

pub(super) struct Analysis {
    pub(super) loading: bool,
    pub(super) generation: u64,
    pub(super) result: Option<Result<Arc<Report>, String>>,
    pub(super) layer: usize,
    findings: bool,
    tree: Entity<TreeState>,
    split: Entity<ResizableState>,
    scroll: UniformListScrollHandle,
    pub(super) rows_rendered: Cell<usize>,
}

impl Analysis {
    pub(super) fn new(cx: &mut Context<FerryDock>) -> Self {
        Self {
            generation: 0,
            loading: false,
            result: None,
            layer: 0,
            findings: false,
            tree: cx.new(|cx| TreeState::new(cx)),
            split: cx.new(|_| ResizableState::default()),
            scroll: UniformListScrollHandle::default(),
            rows_rendered: Cell::new(0),
        }
    }

    pub(super) fn invalidate(&mut self) {
        self.generation += 1;
        self.loading = false;
        self.result = None;
        self.layer = 0;
    }

    pub(super) fn reset(&mut self, cx: &mut Context<FerryDock>) {
        self.generation += 1;
        self.loading = false;
        self.result = None;
        self.layer = 0;
        self.tree
            .update(cx, |tree, cx| tree.set_items(Vec::new(), cx));
        self.scroll = UniformListScrollHandle::default();
    }
}

fn tree_items(nodes: &[FileNode], parent: &str, depth: usize) -> Vec<TreeItem> {
    nodes
        .iter()
        .map(|node| {
            let path = format!("{parent}/{}", node.name);
            TreeItem::new(
                path.clone(),
                format!(
                    "{}  ·  {:.2} MiB  ·  {}",
                    node.name,
                    node.bytes as f64 / 1_048_576.0,
                    node.change
                ),
            )
            .children(tree_items(&node.children, &path, depth + 1))
            .expanded(depth == 0)
        })
        .collect()
}

impl FerryDock {
    pub(super) fn request_image_analysis(&mut self, cx: &mut Context<Self>) {
        if self.connection.is_none() || self.images.details.analysis.loading {
            return;
        }
        let Some(id) = self.images.details.id.clone() else {
            return;
        };
        self.images.details.analysis.reset(cx);
        self.images.details.analysis.loading = true;
        self.backend.send(Command::AnalyzeImage {
            id,
            generation: self.images.details.analysis.generation,
        });
        cx.notify();
    }

    fn cancel_image_analysis(&mut self, cx: &mut Context<Self>) {
        self.backend.send(Command::CloseImageAnalysis);
        // Separate generation from History/Inspect so cancelling cannot reject
        // their pending responses. The analysis request counter is independent.
        self.images.details.analysis.generation += 1;
        self.images.details.analysis.loading = false;
        cx.notify();
    }

    pub(super) fn image_analysis_event(&mut self, event: Event, cx: &mut Context<Self>) {
        if let Event::ImageAnalysis {
            id,
            generation,
            result,
        } = event
            && self.images_open
            && self.connection.is_some()
            && self.images.details.id.as_ref() == Some(&id)
            && self.images.details.analysis.generation == generation
            && self.images.details.analysis.loading
        {
            self.images.details.analysis.loading = false;
            self.images.details.analysis.result = Some(result);
            self.select_analysis_layer(0, cx);
        }
    }

    fn select_analysis_layer(&mut self, index: usize, cx: &mut Context<Self>) {
        let state = &mut self.images.details.analysis;
        state.layer = index;
        let items = state
            .result
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .and_then(|report| report.layers.get(index))
            .map(|layer| tree_items(&layer.files, "", 0))
            .unwrap_or_default();
        state.tree.update(cx, |tree, cx| tree.set_items(items, cx));
        cx.notify();
    }

    pub(super) fn image_analysis_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let state = &self.images.details.analysis;
        let toolbar = div()
            .px_5()
            .py_2()
            .flex()
            .items_center()
            .gap_3()
            .child(
                Self::button(
                    "image-analyze",
                    if state.result.is_some() {
                        "Reanalyze"
                    } else {
                        "Analyze image"
                    },
                    state.loading,
                    self.connection.is_none(),
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.request_image_analysis(cx))),
            )
            .when(state.loading, |view| {
                view.child(
                    Self::button("image-analysis-cancel", "Cancel", false, false, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_image_analysis(cx))),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(colors.muted))
                    .child(if state.loading {
                        "Exporting and analyzing local layers…"
                    } else {
                        "Local image · read-only analysis"
                    }),
            );
        let content = if self.connection.is_none() {
            empty_state(
                "Analysis unavailable",
                "Reconnect to Docker to analyze this image.",
            )
            .into_any_element()
        } else if state.loading {
            empty_state(
                "Analyzing image…",
                "Large images can take a while. You can switch tabs or cancel.",
            )
            .into_any_element()
        } else {
            match &state.result {
            None => empty_state("Explore image layers", "Find large files and bytes retained by files replaced or deleted in later layers.").into_any_element(),
            Some(Err(error)) => Alert::error("image-analysis-error", error.clone()).into_any_element(),
            Some(Ok(report)) => {
                let report = report.clone();
                let selected = state.layer;
                let files = report.layers.get(selected).is_some_and(|layer| !layer.files.is_empty());
                let summary = format!("{} layer{}  ·  {:.2} MiB unpacked  ·  {:.2} MiB estimated waste  ·  {} efficiency",
                    report.layers.len(), if report.layers.len() == 1 { "" } else { "s" }, report.unpacked_bytes as f64 / 1_048_576.0,
                    report.wasted_bytes as f64 / 1_048_576.0, report.efficiency.map(|v| format!("{v}%")).unwrap_or_else(|| "Unavailable".into()));
                let layers = report.clone();
                let layer_list = uniform_list("image-analysis-layers", layers.layers.len(), cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    this.images.details.analysis.rows_rendered.set(range.len());
                    range.filter_map(|index| layers.layers.get(index).map(|layer| (index, layer))).map(|(index, layer)| {
                        ListItem::new(SharedString::from(format!("analysis-layer-entry-{index}")))
                            .selected(index == selected).h(px(62.0)).px_3()
                            .child(div().id(SharedString::from(format!("analysis-layer-{index}"))).test_support().flex_1().min_w_0().flex().flex_col().gap_1()
                                .child(div().text_xs().child(format!("#{} · {}", index + 1,
                                    if layer.metadata_only { "Metadata only".into() } else { format!("{:.2} MiB unpacked", layer.unpacked_bytes as f64 / 1_048_576.0) })))
                                .child(div().text_sm().text_ellipsis().child(layer.command.clone())))
                            .on_click(cx.listener(move |this, _, _, cx| this.select_analysis_layer(index, cx))).into_any_element()
                    }).collect::<Vec<_>>()
                })).track_scroll(&state.scroll).size_full();
                let tabs = TabBar::new("analysis-content-tabs").small().underline()
                    .selected_index(usize::from(state.findings))
                    .children([Tab::new().label("Files"), Tab::new().label(format!("Findings ({})", report.findings.len()))])
                    .on_click(cx.listener(|this, index: &usize, _, cx| { this.images.details.analysis.findings = *index == 1; cx.notify(); }));
                let right = if state.findings {
                    div().id("analysis-findings").test_support().size_full().overflow_y_scroll().px_3().py_2().flex().flex_col().gap_3()
                        .when(report.findings.is_empty(), |view| view.child("No optimization findings."))
                        .children(report.findings.iter().map(|finding| div().flex().flex_col().gap_1().border_b_1().border_color(rgb(colors.border)).pb_2()
                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(format!("{}{}", finding.title, if finding.heuristic { " · Heuristic" } else { "" })))
                            .child(div().text_xs().text_color(rgb(colors.muted)).child(finding.detail.clone()))
                            .when(!finding.hint.is_empty(), |view| view.child(div().text_xs().child(finding.hint.clone())))))
                        .into_any_element()
                } else if !files {
                    empty_state("No file changes", "This layer has no filesystem changes.").into_any_element()
                } else {
                    Tree::new(&state.tree, move |index, entry, _, _, _| {
                        ListItem::new(SharedString::from(format!("analysis-file-entry-{index}")))
                            .pl(px(8.0 + entry.depth() as f32 * 14.0)).pr_2().h(px(28.0))
                            .child(div().w_full().flex().items_center().gap_1()
                                .child(div().text_xs().child(if entry.is_folder() { if entry.is_expanded() { "▾" } else { "▸" } } else { " " }))
                                .child(div().id(SharedString::from(format!("analysis-file-{index}"))).test_support().flex_1().min_w_0().text_xs().text_ellipsis().child(entry.item().label.clone())))
                    }).size_full().into_any_element()
                };
                div().size_full().flex().flex_col()
                    .child(div().id("analysis-summary").test_support().px_5().pb_1().text_sm().child(summary))
                    .child(div().px_5().pb_2().text_xs().text_color(rgb(colors.muted)).child("Waste counts replaced/deleted file payloads, excluding tar overhead. It estimates build savings, not Docker disk recovery. Duplicate-content scanning is not enabled."))
                    .child(div().flex_1().min_h_0().child(h_resizable("analysis-split").with_state(&state.split)
                        .child(resizable_panel().size(px(270.0)).fixed().size_range(px(180.0)..px(500.0))
                            .child(div().id("analysis-layer-pane").test_support().size_full().child(layer_list)))
                        .child(resizable_panel().child(div().size_full().flex().flex_col().child(tabs).child(div().flex_1().min_h_0().child(right))))
                        ))
                    .into_any_element()
            }
        }
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}
