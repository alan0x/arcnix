//! Optional explanations stay beside their controls instead of filling the page.
use glory::prelude::*;

use crate::i18n;

fn container(topic: &'static str, body: impl Widget + 'static) -> impl Widget {
    row().class("help-tip").fill((
        sapid::button()
            .class("help-trigger")
            .name(i18n::bind(format!("Help: {topic}")))
            .fill(text().content("?")),
        col()
            .class("help-copy")
            .fill((text().class("help-title").content(i18n::bind(topic)), body)),
    ))
}

pub(crate) fn tip(topic: &'static str, explanation: impl Into<Vary<String>>) -> impl Widget {
    container(topic, text().content(i18n::bind(explanation)))
}

pub(crate) fn tip_raw(topic: &'static str, explanation: impl Into<Vary<String>>) -> impl Widget {
    container(topic, text().content(explanation))
}

pub(crate) fn group(topic: &'static str, messages: &'static [&'static str]) -> impl Widget {
    container(
        topic,
        each(messages.to_vec())
            .key(|message| *message)
            .children(|item| text().content(i18n::bind(item.value.get()))),
    )
}
