//! Shared elapsed-time wording for resource lists.
pub(crate) fn label(created: Option<i64>, now: i64) -> String {
    let Some(created) = created else {
        return "Unavailable".into();
    };
    let seconds = now.abs_diff(created);
    let Some((scale, unit)) = [
        (365 * 86_400, "year"),
        (30 * 86_400, "month"),
        (86_400, "day"),
        (3_600, "hour"),
        (60, "minute"),
    ]
    .into_iter()
    .find(|(scale, _)| seconds >= *scale) else {
        return "Just now".into();
    };
    let count = seconds / scale;
    let age = format!("{count} {unit}{}", if count == 1 { "" } else { "s" });
    if created > now {
        format!("In {age}")
    } else {
        format!("{age} ago")
    }
}
