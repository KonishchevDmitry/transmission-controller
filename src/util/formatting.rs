pub fn format_error(err: &anyhow::Error) -> String {
    let message = format!("{err:#}");
    let add_dot = !message.ends_with('.') && !message.contains('\n');

    let mut buf = if let mut chars = message.chars() && let Some(first) = chars.next() && first.is_lowercase() {
        let mut buf = String::with_capacity(message.len() + add_dot as usize);
        buf.extend(first.to_uppercase());
        buf.extend(chars);
        buf
    } else {
        message
    };

    if add_dot {
        buf.push('.');
    }

    buf
}