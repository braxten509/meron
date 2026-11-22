//! Builds invitation replies and applies attendance choices.
//! Desktop and mobile invoke these operations for a cached invitation message.
//! Sending the reply and updating the local calendar use the same response value.

use super::*;

/// The `METHOD:REPLY` calendar for `response`: the organizer's event pared to
/// what RFC 5546 3.2.3 asks of a reply — UID, SEQUENCE, the occurrence, the
/// organizer, and the one attendee answering — plus the VTIMEZONEs its times
/// are written in.
pub fn reply_ics(
    invite: &Invite,
    attendee: &Person,
    response: Response,
    now: i64,
) -> Result<String> {
    let original = parse_ical(&invite.ics).ok_or_else(|| anyhow!("invitation is not iCalendar"))?;
    let index = primary_event(&original).ok_or_else(|| anyhow!("invitation has no event"))?;

    let kept = [
        ICalendarProperty::Uid,
        ICalendarProperty::Sequence,
        ICalendarProperty::RecurrenceId,
        ICalendarProperty::Dtstart,
        ICalendarProperty::Dtend,
        ICalendarProperty::Duration,
        ICalendarProperty::Summary,
        ICalendarProperty::Location,
        ICalendarProperty::Organizer,
    ];
    let mut event = ICalendarComponent::new(ICalendarComponentType::VEvent);
    event.entries = original.components[index]
        .entries
        .iter()
        .filter(|entry| kept.contains(&entry.name))
        .cloned()
        .collect();
    event.add_dtstamp(PartialDateTime::from_utc_timestamp(now));
    let mut params = vec![ICalendarParameter::partstat(response.partstat())];
    if !attendee.name.trim().is_empty() {
        params.push(ICalendarParameter::cn(attendee.name.trim().to_string()));
    }
    event.add_property_with_params(
        ICalendarProperty::Attendee,
        params,
        Uri::Location(format!("mailto:{}", attendee.addr.trim())),
    );

    let mut calendar = ICalendarComponent::new(ICalendarComponentType::VCalendar);
    calendar.add_property(
        ICalendarProperty::Prodid,
        ICalendarValue::Text("-//Meron//Meron//EN".to_string()),
    );
    calendar.add_property(
        ICalendarProperty::Version,
        ICalendarValue::Text("2.0".to_string()),
    );
    calendar.add_property(
        ICalendarProperty::Method,
        ICalendarValue::Method(ICalendarMethod::Reply),
    );
    calendar.component_ids = vec![1];
    let mut reply = ICalendar {
        components: vec![calendar, event],
    };
    reply.copy_timezones(&original);
    Ok(reply.to_string())
}

pub fn reply_mail(
    invite: &Invite,
    attendee: &Person,
    response: Response,
    now: i64,
) -> Result<ReplyMail> {
    let who = if attendee.name.trim().is_empty() {
        attendee.addr.trim()
    } else {
        attendee.name.trim()
    };
    let summary = if invite.summary.is_empty() {
        "(no title)"
    } else {
        invite.summary.as_str()
    };
    let to = if invite.organizer.name.is_empty() {
        invite.organizer.addr.clone()
    } else {
        format!(
            "\"{}\" <{}>",
            invite
                .organizer
                .name
                .replace('\\', "\\\\")
                .replace('"', "\\\""),
            invite.organizer.addr
        )
    };
    Ok(ReplyMail {
        to,
        subject: format!("{}: {summary}", response.subject_prefix()),
        body: format!(
            "{who} has {} this invitation: {summary}\r\n",
            response.verb()
        ),
        ics: reply_ics(invite, attendee, response, now)?,
    })
}

/// Send an attendance reply and persist the response for this invitation.
///
/// Answer the invitation in a cached message: send the organizer the reply, remember the answer, and hand
/// back the sent message so the host can file its Sent copy the way it does for any other send.
///
/// `message` is the thread message's `id` (`account#folder#uid`, see
/// [`crate::mail_model::format_thread_id`]), which both clients hold, and `thread_id` its conversation's,
/// which the calendar event links back to.
pub async fn respond(
    engine: &std::sync::Arc<crate::engine::Engine>,
    message: &str,
    thread_id: &str,
    response: Response,
    media_root: &std::path::Path,
) -> Result<Responded> {
    let (account, folder, uid) =
        parse_message_ref(message).ok_or_else(|| anyhow!("invalid message id: {message}"))?;
    let (account, folder) = (account.as_str(), folder.as_str());
    let (message, mine) = {
        let db = engine.db.lock().unwrap();
        (
            crate::store::get_cached_message(&db, account, folder, uid)?,
            crate::store::self_addrs(&db, account),
        )
    };
    let message = message.ok_or_else(|| anyhow!("message is not downloaded"))?;
    let invite = message
        .invite
        .clone()
        .or_else(|| invite_from_attachments(media_root, &message.attachments))
        .ok_or_else(|| anyhow!("message has no invitation"))?;
    if !can_respond(&invite, &mine) {
        return Err(anyhow!("this invitation cannot be answered"));
    }

    let creds = engine.ensure_valid_creds(account).await?;
    // Answer as the address the organizer invited, when the account can send
    // from it; an invite addressed to a list or an unconfigured alias falls
    // back to the account's own address, which organizers accept as a new
    // attendee answering.
    let invited = my_attendee(&invite, &mine).cloned();
    let (from_addr, sender_name) = {
        let db = engine.db.lock().unwrap();
        let requested = invited.as_ref().map(|attendee| attendee.addr.as_str());
        match requested.map(|addr| crate::store::resolve_send_from(&db, account, &creds.user, addr))
        {
            Some(Ok(from)) => from,
            _ => crate::store::resolve_send_from(&db, account, &creds.user, "")?,
        }
    };
    let attendee = Person {
        name: if sender_name.trim().is_empty() {
            invited.map(|attendee| attendee.name).unwrap_or_default()
        } else {
            sender_name.clone()
        },
        addr: from_addr.clone(),
    };
    let mail = reply_mail(&invite, &attendee, response, crate::store::now_unix())?;
    let raw = crate::smtp::send_invite_reply(
        &creds,
        &from_addr,
        &sender_name,
        &mail,
        &message.message_id,
        &message.references,
    )
    .await?;
    {
        let db = engine.db.lock().unwrap();
        crate::store::record_invite_response(
            &db,
            account,
            &invite.uid,
            invite.recurrence_id,
            response.as_str(),
            invite.sequence,
        )?;
        if response == Response::Declined && invite.this_and_future {
            crate::store::mark_this_and_future(&db, account, &invite.uid, invite.recurrence_id)?;
        }
        events::record_invite_event(
            &db,
            account,
            thread_id,
            &message.message_id,
            &invite,
            response,
        )?;
        events::apply_cached_updates(&db, account, &invite.uid)?;
    }
    let event_id = filed_event_id(&engine.db.lock().unwrap(), account, &invite);
    Ok(Responded {
        account: account.to_string(),
        raw,
        event_id,
    })
}

/// Split a per-message id (`account#folder#uid`). The folder may itself hold
/// `#`, so the account ends at the first and the uid starts after the last.
pub(super) fn parse_message_ref(id: &str) -> Option<(String, String, u32)> {
    let (account, rest) = id.split_once('#')?;
    let (folder, uid) = rest.rsplit_once('#')?;
    if account.is_empty() || folder.is_empty() {
        return None;
    }
    Some((
        account.to_string(),
        crate::mail_model::canon_folder(folder),
        uid.parse().ok()?,
    ))
}

/// Take a cancelled invitation out of the calendar, as a decline would but
/// with nothing sent: the occurrence it names, or the whole event when it
/// cancels the series. `message` is the thread message's `id`.
pub fn remove_from_calendar(
    conn: &rusqlite::Connection,
    message: &str,
    media_root: &std::path::Path,
) -> Result<()> {
    let (account, folder, uid) =
        parse_message_ref(message).ok_or_else(|| anyhow!("invalid message id: {message}"))?;
    let cached = crate::store::get_cached_message(conn, &account, &folder, uid)?
        .ok_or_else(|| anyhow!("message is not downloaded"))?;
    let invite = cached
        .invite
        .clone()
        .or_else(|| invite_from_attachments(media_root, &cached.attachments))
        .ok_or_else(|| anyhow!("message has no invitation"))?;
    // Remembered like a decline, so a series accepted later — which folds in
    // the occurrences answered on their own — keeps this one out.
    if invite.recurrence_id != 0 {
        crate::store::record_invite_response(
            conn,
            &account,
            &invite.uid,
            invite.recurrence_id,
            Response::Declined.as_str(),
            invite.sequence,
        )?;
        if invite.this_and_future {
            crate::store::mark_this_and_future(conn, &account, &invite.uid, invite.recurrence_id)?;
        }
    }
    events::record_invite_event(conn, &account, "", "", &invite, Response::Declined)
}

/// Put an invitation the user already answered back in the calendar — after
/// its event was deleted — with the answer they gave and nothing sent.
pub fn add_to_calendar(
    conn: &rusqlite::Connection,
    message: &str,
    thread_id: &str,
    media_root: &std::path::Path,
) -> Result<String> {
    let (account, folder, uid) =
        parse_message_ref(message).ok_or_else(|| anyhow!("invalid message id: {message}"))?;
    let cached = crate::store::get_cached_message(conn, &account, &folder, uid)?
        .ok_or_else(|| anyhow!("message is not downloaded"))?;
    let invite = cached
        .invite
        .clone()
        .or_else(|| invite_from_attachments(media_root, &cached.attachments))
        .ok_or_else(|| anyhow!("message has no invitation"))?;
    let recorded =
        crate::store::invite_response(conn, &account, &invite.uid, invite.recurrence_id)?;
    let mine = crate::store::self_addrs(conn, &account);
    let response = Response::parse(&my_response(&invite, &mine, recorded.as_ref()))
        .filter(|response| *response != Response::Declined)
        .ok_or_else(|| anyhow!("this invitation has not been accepted"))?;
    events::record_invite_event(
        conn,
        &account,
        thread_id,
        &cached.message_id,
        &invite,
        response,
    )?;
    events::apply_cached_updates(conn, &account, &invite.uid)?;
    Ok(filed_event_id(conn, &account, &invite))
}
