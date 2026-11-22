//! MIME parsing helpers built on `mailparse`.
//!
//! Two jobs: decode RFC 2047 encoded-words in bare header fragments, and turn
//! a full RFC822 message into a readable summary + body for the reader view.

use base64::engine::general_purpose::STANDARD;
use html_to_markdown_rs::{ConversionOptions, convert};
use mailparse::body::Body;
use mailparse::{
    DispositionType, MailAddr, MailAddrList, MailHeader, MailHeaderMap, ParsedMail, addrparse,
    addrparse_header, parse_header, parse_mail,
};
use std::path::{Path, PathBuf};

mod media;
use media::collect_attachments;
#[cfg(test)]
pub(crate) use media::test_media_root;
#[cfg(test)]
use media::write_media_with_cap;
pub use media::{
    MediaCtx, cached_media_available, media_root, missing_media_count, prune_media_cache,
    remove_account_media, remove_folder_media, remove_message_media,
};
pub(crate) use media::{is_attachment_part, sanitize_segment};
mod envelope;
pub use envelope::{SentEnvelope, normalize_addresses, parse_date_to_epoch, sent_envelope_of};
use envelope::{collect_address_list, normalize_msgid, normalize_references};
mod message;
pub use message::{Attachment, Message, message_id_of, parse_message};
pub(crate) use message::{is_calendar_mime, parse_message_unpruned};
mod addresses;
use addresses::from_of;
pub use addresses::{addrparse_lenient, decode_words, split_address};
mod html;
use html::rewrite_cid_refs;
pub use html::{prepare_html, prepare_message_html, render_body};
mod text;
pub use text::html_to_text;
use text::normalize_text;
mod body;
#[cfg(test)]
use body::plain_is_markup;
use body::{body_sources, text_body};
mod preview;
pub(crate) use preview::{preview_of, text_start};
#[cfg(test)]
mod tests;
