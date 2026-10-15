use super::*;
use crate::app::ui::palette;
use gpui_kit::{
    base::TestSupportExt,
    component::{
        dialog::Dialog,
        input::{Input, Textarea},
        select::Select,
    },
};

impl FerryDock {
    pub(super) fn favorites_dialog(&self, dialog: Dialog, cx: &mut Context<Self>) -> Dialog {
        let colors = palette(cx);
        let owner = cx.entity().downgrade();
        let mut body = div()
            .flex()
            .flex_col()
            .gap_3()
            .when_some(self.favorites.focus.as_ref(), |body, focus| {
                body.track_focus(focus)
            })
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    window.close_dialog(cx);
                    this.close_favorites(window, cx);
                    cx.stop_propagation();
                }
            }));
        let mut footer = div().flex().justify_end().gap_2();
        let waiting = self.favorites.waiting;
        let available = self.favorites.loaded && !waiting;
        if let Some((endpoint, target)) = &self.favorites.scope {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(rgb(colors.muted))
                    .text_ellipsis()
                    .child(format!("{} · {endpoint}", target.label())),
            );
        }
        if waiting {
            body = body.child("Loading / saving favorite commands…");
        }
        if let Some(error) = &self.favorites.error {
            body = body.child(div().text_color(rgb(colors.error)).child(error.clone()));
            if !self.favorites.loaded {
                body = body.child(
                    Self::button("favorites-retry", "Retry loading", false, waiting, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.mutate_favorites(None, cx))),
                );
            }
        }
        if let Some(review) = &self.favorites.review {
            body = body.child(div().font_weight(FontWeight::MEDIUM).child(review.favorite.label.clone()))
                .child("Command to run inside the selected container:")
                .child(div().id("favorite-command-preview").test_support().h(px(100.0)).flex_none().overflow_y_scroll().font_family("monospace").text_sm().child(review.favorite.command.clone()))
                .child(Select::new(&review.picker).placeholder("Choose container instance…").w_full().disabled(review.instances.is_empty()).id("favorite-instance"))
                .child("Runs once in a new interactive terminal using sh. Output remains visible after exit.");
            let runnable = match self.favorite_run_target(cx) {
                Ok(_) => true,
                Err(message) => {
                    body = body.child(div().text_sm().text_color(rgb(colors.muted)).child(message));
                    false
                }
            };
            footer = footer.child(
                Self::button("favorite-run", "Run in terminal", false, !runnable, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.run_favorite(window, cx))),
            );
        } else if let Some(editor) = &self.favorites.editor {
            body = body
                .child("Label")
                .child(
                    div()
                        .id("favorite-label")
                        .test_support()
                        .child(Input::new(&editor.label).disabled(waiting)),
                )
                .child("Command (sh syntax; multiline commands supported)")
                .child(
                    div()
                        .id("favorite-command")
                        .test_support()
                        .child(Textarea::new(&editor.command).disabled(waiting)),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(colors.muted))
                        .child("Saved on this computer. Saving never executes the command."),
                );
            footer = footer.child(
                Self::button("favorite-save", "Save favorite", false, !available, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.save_favorite(cx))),
            );
        } else {
            if let Some(picker) = &self.favorites.picker {
                body = body.child(
                    Select::new(picker)
                        .placeholder("Choose service…")
                        .w_full()
                        .disabled(!available)
                        .id("favorite-service"),
                );
            }
            let indices = self
                .favorites
                .scope
                .as_ref()
                .map(|(endpoint, target)| {
                    self.favorites
                        .items
                        .iter()
                        .enumerate()
                        .filter_map(|(index, f)| {
                            (&f.endpoint == endpoint && &f.target == target).then_some(index)
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let count = indices.len();
            let recipes = self.favorites.items.clone();
            if count == 0 && self.favorites.loaded {
                body = body.child("No favorite commands for this target.");
            }
            if count > 0 {
                body = body.child(
                    uniform_list(
                        "favorite-list",
                        count,
                        cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                            range
                                .filter_map(|index| {
                                    indices.get(index).and_then(|index| recipes.get(*index))
                                })
                                .map(|recipe| {
                                    let id = recipe.id;
                                    div()
                                        .w_full()
                                        .h(px(62.0))
                                        .min_h(px(62.0))
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .border_b_1()
                                        .border_color(rgb(colors.border))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .child(
                                                    div()
                                                        .text_ellipsis()
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .child(recipe.label.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .text_color(rgb(colors.muted))
                                                        .text_ellipsis()
                                                        .child(recipe.command.replace('\n', " · ")),
                                                ),
                                        )
                                        .child(
                                            Self::button(
                                                SharedString::from(format!("favorite-review-{id}")),
                                                "Run…",
                                                false,
                                                !available,
                                                cx,
                                            )
                                            .on_click(
                                                cx.listener(move |this, _, window, cx| {
                                                    this.review_favorite(id, window, cx)
                                                }),
                                            ),
                                        )
                                        .child(
                                            Self::button(
                                                SharedString::from(format!("favorite-edit-{id}")),
                                                "Edit",
                                                false,
                                                !available,
                                                cx,
                                            )
                                            .on_click(
                                                cx.listener(move |this, _, window, cx| {
                                                    this.edit_favorite(Some(id), window, cx)
                                                }),
                                            ),
                                        )
                                        .child(
                                            Self::button(
                                                SharedString::from(format!("favorite-delete-{id}")),
                                                "Delete",
                                                false,
                                                !available,
                                                cx,
                                            )
                                            .on_click(
                                                cx.listener(move |this, _, _, cx| {
                                                    this.mutate_favorites(
                                                        Some(Mutation::Delete(id)),
                                                        cx,
                                                    )
                                                }),
                                            ),
                                        )
                                        .into_any_element()
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .flex_none()
                    .w_full()
                    .h(px(62.0 * count.clamp(1, 4) as f32)),
                );
            }
            let target_selected =
                self.favorites
                    .scope
                    .as_ref()
                    .is_some_and(|(_, target)| match target {
                        Target::Service { service, .. } => !service.is_empty(),
                        _ => true,
                    });
            footer = footer.child(
                Self::button(
                    "favorite-add",
                    "Add command",
                    false,
                    !available || !target_selected,
                    cx,
                )
                .on_click(cx.listener(|this, _, window, cx| this.edit_favorite(None, window, cx))),
            );
        }
        if self.favorites.editor.is_some() || self.favorites.review.is_some() {
            footer = footer.child(
                Self::button("favorite-back", "Back", false, waiting, cx).on_click(cx.listener(
                    |this, _, window, cx| {
                        this.favorites.editor = None;
                        this.favorites.review = None;
                        if let Some(focus) = &this.favorites.focus {
                            focus.focus(window, cx);
                        }
                        cx.notify();
                    },
                )),
            );
        }
        footer = footer.child(
            Self::button("favorites-close", "Close", false, waiting, cx).on_click(cx.listener(
                |this, _, window, cx| {
                    window.close_dialog(cx);
                    this.close_favorites(window, cx);
                },
            )),
        );
        dialog
            .title("Favorite commands")
            .width(px(670.0))
            .overlay_closable(false)
            // Enter/Escape in fields must not launch a command or leak to app shortcuts.
            .on_ok(|_, _, _| false)
            .on_cancel(move |_, window, cx| {
                owner
                    .update(cx, |this, cx| {
                        this.close_favorites(window, cx);
                        true
                    })
                    .unwrap_or(true)
            })
            .on_close(cx.listener(|this, _, window, cx| this.close_favorites(window, cx)))
            .child(body)
            .footer(footer)
    }
}
