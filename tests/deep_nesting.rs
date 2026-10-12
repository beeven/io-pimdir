//! Derivations over bodies nested to exhaust the stack.
//!
//! A sender controls a message's MIME tree and an organizer a calendar's
//! components, so neither depth may reach the stack. Each case runs on a
//! 1 MiB thread, the stack an ordinary Android thread gets, where an
//! unbounded walk overflowed at about 5000 multipart levels (350 KB).

use std::thread;

use io_pimdir::summary::{self, PimdirSummary, mail};

/// Run a case on a 1 MiB stack.
fn on_small_stack(case: impl FnOnce() + Send + 'static) {
    thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(case)
        .unwrap()
        .join()
        .unwrap();
}

/// A message nesting `depth` `multipart/mixed` levels around a leaf part
/// carrying `leaf` as its headers.
fn nested_mail(depth: usize, leaf: &str) -> Vec<u8> {
    let mut out = String::from("Message-ID: <deep@example.org>\r\n");

    // NOTE: Each boundary ends in `x`, so none is a prefix of another.
    for level in 0..depth {
        out.push_str(&format!(
            "Content-Type: multipart/mixed; boundary=b{level}x\r\n\r\n--b{level}x\r\n"
        ));
    }

    out.push_str(leaf);
    out.push_str("\r\n\r\nleaf\r\n");

    for level in (0..depth).rev() {
        out.push_str(&format!("--b{level}x--\r\n"));
    }

    out.into_bytes()
}

/// The walked attachment mark and invitation of a message.
fn walked(body: &[u8]) -> (Option<bool>, Option<String>) {
    match mail::derive(body).summary {
        Some(PimdirSummary::Mail(mail)) => (mail.attachment, mail.invitation),
        _ => panic!("a mail summary"),
    }
}

#[test]
fn a_deep_multipart_tree_derives_within_the_stack() {
    on_small_stack(|| {
        let attached = "Content-Disposition: attachment; filename=a.pdf";
        assert_eq!(walked(&nested_mail(10_000, attached)), (Some(false), None));

        let invite =
            "Content-Type: text/calendar\r\n\r\nBEGIN:VCALENDAR\r\nUID:deep\r\nEND:VCALENDAR";
        assert_eq!(walked(&nested_mail(10_000, invite)), (Some(false), None));
    });
}

#[test]
fn the_walk_reaches_parts_down_to_the_bound() {
    let attached = "Content-Disposition: attachment; filename=a.pdf";
    let depth = mail::MIME_DEPTH_MAX;

    assert_eq!(walked(&nested_mail(depth, attached)).0, Some(true));
    assert_eq!(walked(&nested_mail(depth + 1, attached)).0, Some(false));

    let invite = "Content-Type: text/calendar\r\n\r\nBEGIN:VCALENDAR\r\nUID:deep\r\nEND:VCALENDAR";
    assert_eq!(
        walked(&nested_mail(depth, invite)).1.as_deref(),
        Some("deep")
    );
    assert_eq!(walked(&nested_mail(depth + 1, invite)).1, None);
}

#[test]
fn a_deep_component_tree_derives_within_the_stack() {
    on_small_stack(|| {
        let mut body = String::from(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:e1\r\nDTSTART:20260801T120000Z\r\nSUMMARY:Deep\r\n",
        );
        for _ in 0..200_000 {
            body.push_str("BEGIN:X\r\n");
        }
        for _ in 0..200_000 {
            body.push_str("END:X\r\n");
        }
        body.push_str("END:VEVENT\r\nEND:VCALENDAR\r\n");

        let derivation = summary::calendar::derive(body.as_bytes());
        assert_eq!(derivation.link_id.0, "e1");
        assert!(!summary::calendar::scheduled(body.as_bytes()));
    });
}
