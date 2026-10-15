//! Networks workspace: searchable inventory, configured endpoints and routing.
use super::{
    FerryDock,
    ui::{empty_state, palette},
};
use crate::{
    docker::{Command, Event},
    networks::{Member, Network},
};
use gpui::{prelude::*, *};
use gpui_kit::{
    base::TestSupportExt,
    component::{
        Sizable,
        alert::Alert,
        input::{Input, InputEvent, InputState},
        list::ListItem,
        resizable::{ResizableState, resizable_panel, v_resizable},
        scroll::{Scrollbar, ScrollbarMode},
        spinner::Spinner,
        tag::Tag,
        toolbar::Toolbar,
    },
};
use std::{cell::Cell, collections::HashSet};

pub(super) struct Networks {
    endpoint: Option<String>,
    container_ids: HashSet<String>,
    pub(super) items: Vec<Network>,
    pub(super) visible: Vec<usize>,
    pub(super) selected: Option<String>,
    filter: String,
    pub(super) container_filter: Option<(String, String)>,
    search: Entity<InputState>,
    search_focus: FocusHandle,
    pub(super) generation: u64,
    pub(super) detail_generation: u64,
    pub(super) details: Option<Network>,
    loading: bool,
    detail_loading: bool,
    error: Option<String>,
    detail_error: Option<String>,
    scroll: UniformListScrollHandle,
    member_scroll: UniformListScrollHandle,
    dock: Entity<ResizableState>,
    pub(super) rows_rendered: Cell<usize>,
    pub(super) members_rendered: Cell<usize>,
}

impl Networks {
    pub(super) fn new(window: &mut Window, cx: &mut Context<FerryDock>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search name, driver, project or subnet…  Ctrl+F")
        });
        let search_focus = search.focus_handle(cx);
        cx.subscribe(&search, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.networks.filter = input.read(cx).value().to_lowercase();
                this.networks.filter_items();
                this.networks.scroll.scroll_to_item(0, ScrollStrategy::Top);
                cx.notify();
            }
        })
        .detach();
        let dock = cx.new(|_| ResizableState::default());
        cx.observe(&dock, |_, _, cx| cx.notify()).detach();
        Self {
            endpoint: None,
            container_ids: HashSet::new(),
            items: Vec::new(),
            visible: Vec::new(),
            selected: None,
            filter: String::new(),
            container_filter: None,
            search,
            search_focus,
            generation: 0,
            detail_generation: 0,
            details: None,
            loading: false,
            detail_loading: false,
            error: None,
            detail_error: None,
            scroll: UniformListScrollHandle::default(),
            member_scroll: UniformListScrollHandle::default(),
            dock,
            rows_rendered: Cell::new(0),
            members_rendered: Cell::new(0),
        }
    }

    fn filter_items(&mut self) {
        self.visible = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, network)| {
                (network.matches(&self.filter)
                    && self.container_filter.as_ref().is_none_or(|(id, _)| {
                        network.members.iter().any(|member| &member.id == id)
                    }))
                .then_some(index)
            })
            .collect();
    }
}

impl FerryDock {
    pub(super) fn show_networks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_images(false, window, cx);
        self.networks.container_filter = None;
        self.networks.filter_items();
        self.networks_open = true;
        self.refresh_networks();
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn show_container_networks(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self
            .containers
            .items
            .iter()
            .find(|container| container.id == id)
            .map(|container| container.name.clone())
        else {
            return;
        };
        self.show_networks(window, cx);
        self.networks.container_filter = Some((id, name));
        self.networks.selected = None;
        self.networks.filter.clear();
        self.networks
            .search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.networks.filter_items();
        self.request_network_details();
        cx.notify();
    }

    pub(super) fn close_networks(&mut self) {
        if !self.networks_open {
            return;
        }
        self.networks_open = false;
        self.networks.generation += 1;
        self.networks.detail_generation += 1;
        self.networks.loading = false;
        self.networks.detail_loading = false;
        self.backend.send(Command::CloseNetworks);
    }

    pub(super) fn refresh_networks(&mut self) {
        self.networks.generation += 1;
        self.networks.loading = true;
        self.backend.send(Command::Networks {
            generation: self.networks.generation,
        });
        self.request_network_details();
    }

    fn request_network_details(&mut self) {
        self.networks.detail_generation += 1;
        self.networks.details = None;
        self.networks.detail_error = None;
        self.networks.detail_loading = self.networks.selected.is_some();
        if let Some(id) = &self.networks.selected {
            self.backend.send(Command::NetworkDetails {
                id: id.clone(),
                generation: self.networks.detail_generation,
            });
        } else {
            self.backend.send(Command::CloseNetworkDetails);
        }
    }

    fn select_network(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.networks.selected.as_ref() != Some(&id) {
            self.networks.selected = Some(id);
            self.networks
                .member_scroll
                .scroll_to_item(0, ScrollStrategy::Top);
            self.request_network_details();
        }
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn networks_container_snapshot(&mut self, containers: &[crate::model::Container]) {
        self.networks.container_ids = containers
            .iter()
            .map(|container| container.id.clone())
            .collect();
    }

    pub(super) fn networks_connected(&mut self, endpoint: &str) {
        if self.networks.endpoint.as_deref() != Some(endpoint) {
            self.networks_disconnected();
            self.networks.selected = None;
            self.networks.container_filter = None;
            self.networks.endpoint = Some(endpoint.to_owned());
        }
    }

    pub(super) fn networks_disconnected(&mut self) {
        self.networks.generation += 1;
        self.networks.detail_generation += 1;
        self.networks.items.clear();
        self.networks.visible.clear();
        self.networks.details = None;
        self.networks.loading = false;
        self.networks.detail_loading = false;
        self.networks.error = None;
        self.networks.detail_error = None;
    }

    pub(super) fn network_event(&mut self, event: Event) {
        match event {
            Event::Networks { generation, result }
                if self.networks_open
                    && self.connection.is_some()
                    && generation == self.networks.generation =>
            {
                self.networks.loading = false;
                match result {
                    Ok(items) => {
                        self.networks.error = None;
                        self.networks.items = items;
                        self.networks.filter_items();
                        if !self.networks.selected.as_ref().is_some_and(|id| {
                            self.networks.items.iter().any(|network| &network.id == id)
                        }) {
                            self.networks.selected = self
                                .networks
                                .visible
                                .first()
                                .map(|index| self.networks.items[*index].id.clone());
                            self.networks
                                .member_scroll
                                .scroll_to_item(0, ScrollStrategy::Top);
                            self.request_network_details();
                        }
                    }
                    Err(error) => self.networks.error = Some(error),
                }
            }
            Event::NetworkDetails {
                id,
                generation,
                result,
            } if self.networks_open
                && self.connection.is_some()
                && generation == self.networks.detail_generation
                && self.networks.selected.as_ref() == Some(&id) =>
            {
                self.networks.detail_loading = false;
                match result {
                    Ok(network) => {
                        self.networks.details = Some(network);
                        self.networks.detail_error = None;
                    }
                    Err(error) => {
                        self.networks.details = None;
                        self.networks.detail_error = Some(error);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn networks_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = &event.keystroke;
        match key.key.as_str() {
            "f" if key.modifiers.control => self.networks.search_focus.focus(window, cx),
            "r" if key.modifiers.control => self.refresh(),
            "up" | "down" if self.focus.is_focused(window) => {
                let current = self.networks.visible.iter().position(|index| {
                    Some(&self.networks.items[*index].id) == self.networks.selected.as_ref()
                });
                let next = match current {
                    Some(index) if key.key == "up" => index.saturating_sub(1),
                    Some(index) => (index + 1).min(self.networks.visible.len().saturating_sub(1)),
                    None => 0,
                };
                if let Some(index) = self.networks.visible.get(next) {
                    self.select_network(self.networks.items[*index].id.clone(), window, cx);
                    self.networks
                        .scroll
                        .scroll_to_item(next, ScrollStrategy::Nearest);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn network_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        if self.networks.visible.is_empty() {
            let (title, description) = if self.connection.is_none() {
                (
                    "Waiting for Docker…",
                    "Networks will reload when Docker reconnects.",
                )
            } else if self.networks.loading {
                (
                    "Loading networks…",
                    "Reading Docker networks and configured container references.",
                )
            } else if self.networks.error.is_some() {
                (
                    "Networks unavailable",
                    "Refresh to try loading the network inventory again.",
                )
            } else if !self.networks.filter.is_empty() {
                (
                    "No networks match your search",
                    "Search by name, driver, Compose project or subnet.",
                )
            } else {
                (
                    "No networks on this Docker context",
                    "Networks appear here after creating them with Docker or Compose.",
                )
            };
            return empty_state(title, description).into_any_element();
        }
        let list = uniform_list(
            "networks-scroll",
            self.networks.visible.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                this.networks.rows_rendered.set(range.len());
                range
                    .filter_map(|row| {
                        this.networks
                            .visible
                            .get(row)
                            .and_then(|index| this.networks.items.get(*index))
                    })
                    .map(|network| {
                        let id = network.id.clone();
                        let selected = this.networks.selected.as_ref() == Some(&id);
                        ListItem::new(SharedString::from(format!("network-row-{id}")))
                            .selected(selected)
                            .aria_selected(selected)
                            .accessibility_label(network.name.clone())
                            .h(px(56.0))
                            .w_full()
                            .px_4()
                            .bg(rgb(if selected {
                                colors.selected
                            } else {
                                colors.background
                            }))
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .child(
                                                div().text_ellipsis().child(network.name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(colors.muted))
                                                    .text_ellipsis()
                                                    .child(
                                                        network
                                                            .project()
                                                            .unwrap_or("No Compose project")
                                                            .to_owned(),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .w(px(90.0))
                                            .text_xs()
                                            .text_ellipsis()
                                            .child(network.driver.clone()),
                                    )
                                    .child(
                                        div()
                                            .w(px(70.0))
                                            .text_xs()
                                            .text_ellipsis()
                                            .child(network.scope.clone()),
                                    )
                                    .child(div().w(px(175.0)).text_xs().text_ellipsis().child(
                                        if network.subnets.is_empty() {
                                            "Unavailable".into()
                                        } else {
                                            network.subnets.join(", ")
                                        },
                                    ))
                                    .child(
                                        div()
                                            .w(px(105.0))
                                            .text_xs()
                                            .child(format!("{} containers", network.members.len())),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_network(id.clone(), window, cx)
                            }))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.networks.scroll)
        .size_full();
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .child(list)
            .child(Scrollbar::vertical(&self.networks.scroll).mode(ScrollbarMode::Always))
            .into_any_element()
    }

    fn open_network_container(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self
            .containers
            .items
            .iter()
            .any(|container| container.id == id)
        {
            Self::notify(
                "Container is unavailable on this Docker context; refresh to check it",
                true,
                window,
                cx,
            );
            return;
        }
        self.show_images(false, window, cx);
        self.containers.running_only = false;
        self.containers.filter.clear();
        self.containers
            .search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.select(id.to_owned(), window, cx);
    }

    fn network_members(&self, network: &Network, cx: &mut Context<Self>) -> AnyElement {
        if network.members.is_empty() {
            return empty_state("No containers configured on this network","Host and none networks do not assign container IP addresses. Shared network namespaces and remote Swarm tasks may not have a local endpoint.").into_any_element();
        }
        let colors = palette(cx);
        let members = network.members.clone();
        let list = uniform_list(
            "network-members-scroll",
            members.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                this.networks.members_rendered.set(range.len());
                range
                    .filter_map(|row| members.get(row))
                    .map(|member| {
                        let id = member.id.clone();
                        let available = this.networks.container_ids.contains(&id);
                        let names = names_label(member);
                        let text = format!(
                            "{} ({})\nIPv4: {}\nIPv6: {}\nAliases / DNS names: {}{}",
                            member.name,
                            member.state,
                            member.ipv4.as_deref().unwrap_or("Unavailable"),
                            member.ipv6.as_deref().unwrap_or("Unavailable"),
                            names,
                            member
                                .error
                                .as_ref()
                                .map(|error| format!("\n{error}"))
                                .unwrap_or_default()
                        );
                        div()
                            .id(SharedString::from(format!("network-member-{id}")))
                            .test_support()
                            .debug_selector({
                                let id = id.clone();
                                move || format!("network-member-{id}")
                            })
                            .h(px(68.0))
                            .w_full()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_4()
                            .border_b_1()
                            .border_color(rgb(colors.border))
                            .child(
                                div()
                                    .w(px(230.0))
                                    .flex_shrink_0()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        Self::button(
                                            SharedString::from(format!("network-open-{id}")),
                                            member.name.clone(),
                                            false,
                                            !available,
                                            cx,
                                        )
                                        .tooltip("Open this container")
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.open_network_container(&id, window, cx)
                                            }),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(colors.muted))
                                            .child(member.state.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(145.0))
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_ellipsis()
                                    .child(
                                        member.ipv4.as_deref().unwrap_or("Unavailable").to_owned(),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(155.0))
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_ellipsis()
                                    .child(
                                        member.ipv6.as_deref().unwrap_or("Unavailable").to_owned(),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .flex()
                                    .flex_col()
                                    .child(div().text_ellipsis().child(names))
                                    .when_some(member.error.clone(), |view, error| {
                                        view.child(
                                            div()
                                                .text_color(rgb(colors.muted))
                                                .text_ellipsis()
                                                .child(error),
                                        )
                                    }),
                            )
                            .child(
                                Self::button(
                                    SharedString::from(format!("network-copy-{}", member.id)),
                                    "Copy",
                                    false,
                                    false,
                                    cx,
                                )
                                .tooltip(text.clone())
                                .on_click(cx.listener(
                                    move |_, _, window, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            text.clone(),
                                        ));
                                        Self::notify("Endpoint details copied", false, window, cx);
                                    },
                                )),
                            )
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.networks.member_scroll)
        .size_full();
        div()
            .flex_1()
            .w_full()
            .min_h_0()
            .relative()
            .overflow_hidden()
            .child(list)
            .child(Scrollbar::vertical(&self.networks.member_scroll).mode(ScrollbarMode::Always))
            .into_any_element()
    }

    fn network_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let mut view = div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(rgb(colors.background));
        if let Some(error) = &self.networks.detail_error {
            view = view.child(
                Alert::error("network-detail-error", error.clone())
                    .title("Network details unavailable"),
            );
        }
        let Some(network) = &self.networks.details else {
            return view
                .child(empty_state(
                    if self.networks.detail_loading {
                        "Loading network details…"
                    } else {
                        "Select a network"
                    },
                    "Container addresses and DNS aliases appear here.",
                ))
                .into_any_element();
        };
        let flags = format!(
            "{} · {} · Internal: {} · IPv6: {}",
            network.driver,
            network.scope,
            bool_label(network.internal),
            bool_label(network.ipv6)
        );
        view.child(Toolbar::new("network-detail-toolbar").h_auto().min_h(px(60.0)).flex_shrink_0().px_4().py_2()
            .content(div().flex_1().min_w_0().flex().flex_col()
                .child(div().font_weight(FontWeight::MEDIUM).text_ellipsis().child(network.name.clone()))
                .child(div().text_xs().text_color(rgb(colors.muted)).child(flags)))
            .content(Tag::secondary().small().outline().child(format!("{} containers",network.members.len()))))
            .child(div().px_4().py_2().text_xs().flex().flex_col().gap_1()
                .child(format!("Subnet (CIDR): {} · Gateway: {}",values_label(&network.subnets),values_label(&network.gateways)))
                .child(div().text_color(rgb(colors.muted)).child("Configured containers include stopped containers. Addresses and DNS names may be unavailable. Click Copy for full endpoint details.")))
            .child(div().px_4().py_2().flex().gap_2().text_xs().text_color(rgb(colors.muted)).border_b_1().border_color(rgb(colors.border))
                .child(div().w(px(230.0)).flex_shrink_0().child("Container / state"))
                .child(div().w(px(145.0)).flex_shrink_0().child("IPv4 (CIDR)"))
                .child(div().w(px(155.0)).flex_shrink_0().child("IPv6 (CIDR)"))
                .child(div().flex_1().child("Aliases / DNS names")))
            .child(self.network_members(network,cx)).into_any_element()
    }

    pub(super) fn networks_workspace(&self, cx: &mut Context<Self>) -> Div {
        let colors = palette(cx);
        let mut view = div().size_full().flex().flex_col().child(
            Toolbar::new("networks-toolbar")
                .px_5()
                .py_3()
                .border_b_1()
                .border_color(rgb(colors.border))
                .content(div().font_weight(FontWeight::MEDIUM).child("Networks"))
                .content(Tag::secondary().small().outline().child(format!(
                    "{} / {} networks",
                    self.networks.visible.len(),
                    self.networks.items.len()
                )))
                .when(self.networks.loading, |toolbar| {
                    toolbar.content(Spinner::new().small())
                })
                .when_some(
                    self.networks.container_filter.as_ref(),
                    |toolbar, (_, name)| {
                        toolbar
                            .content(
                                Tag::secondary()
                                    .small()
                                    .outline()
                                    .child(format!("For {name}")),
                            )
                            .child(
                                Self::button("network-show-all", "All networks", false, false, cx)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.networks.container_filter = None;
                                        this.networks.filter_items();
                                        cx.notify();
                                    })),
                            )
                    },
                )
                .content(div().flex_1())
                .content(
                    div()
                        .id("network-search")
                        .test_support()
                        .w(px(350.0))
                        .capture_key_down(cx.listener(Self::search_key))
                        .child(Input::new(&self.networks.search).small()),
                ),
        );
        if let Some(error) = &self.networks.error {
            view = view.child(
                Alert::error("network-inventory-error", error.clone())
                    .title("Network inventory unavailable"),
            );
        }
        if let Some(error) = &self.connection_error {
            view = view.child(
                Alert::error("network-connection-error", error.clone())
                    .title("Docker connection unavailable"),
            );
        }
        view.child(
            v_resizable("networks-split")
                .with_state(&self.networks.dock)
                .child(
                    resizable_panel()
                        .size_range(px(100.0)..Pixels::MAX)
                        .child(self.network_list(cx)),
                )
                .child(
                    resizable_panel()
                        .size(px(370.0))
                        .size_range(px(250.0)..Pixels::MAX)
                        .child(self.network_detail(cx)),
                ),
        )
    }
}

fn bool_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "Unavailable",
    }
}
fn values_label(values: &[String]) -> String {
    if values.is_empty() {
        "Unavailable".into()
    } else {
        values.join(", ")
    }
}
fn names_label(member: &Member) -> String {
    if member.aliases.is_none() && member.dns_names.is_none() {
        return "Unavailable".into();
    }
    let mut names = member
        .aliases
        .iter()
        .flatten()
        .chain(member.dns_names.iter().flatten())
        .filter(|name| !name.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    if names.is_empty() {
        "None reported".into()
    } else {
        names.join(", ")
    }
}
