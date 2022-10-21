use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, BufReader, BufRead};
use std::path::Path;

use log::debug;

use lettre::{Message, Transport, SmtpTransport};
use lettre::message::Mailbox;

use crate::common::{EmptyResult, GenericResult};

#[derive(Debug)]
pub struct Mailer {
    from: Mailbox,
    to: Mailbox,
}

#[derive(Debug)]
pub struct EmailTemplate {
    subject: String,
    body: String,
}

impl Mailer {
    pub fn new(from: &str, to: &str) -> GenericResult<Mailer> {
        Ok(Mailer {
            from: from.parse().map_err(|_| format!("Invalid email: {:?}", from))?,
            to: to.parse().map_err(|_| format!("Invalid email: {:?}", to))?,
        })
    }

    pub fn send(&self, subject: &str, body: &str) -> EmptyResult {
        let message = Message::builder()
            .from(self.from.clone())
            .to(self.to.clone())
            .subject(subject)
            .body(body.to_owned())
            .map_err(|e| format!("Failed to construct a email: {}", e))?;

        debug!("Sending {:?} email to {}...", subject, self.to.email);
        SmtpTransport::unencrypted_localhost().send(&message)?;
        debug!("The email has been sent.");

        Ok(())
    }
}

impl EmailTemplate {
    pub fn new(subject: &str, body: &str) -> EmailTemplate {
        EmailTemplate {
            subject: s!(subject),
            body: s!(body),
        }
    }

    pub fn new_from_file<P: AsRef<Path>>(path: P) -> GenericResult<EmailTemplate> {
        let mut file = BufReader::new(File::open(path)?);

        let mut subject = String::new();
        file.read_line(&mut subject)?;
        let subject = subject.trim();
        if subject.is_empty() {
            return Err!("The first line must be a non-empty message subject")
        }

        let mut delimiter = String::new();
        file.read_line(&mut delimiter)?;
        if !delimiter.trim_end_matches(|c| c == '\r' || c == '\n').is_empty() {
            return Err!("The second line must be an empty delimiter between message subject and body")
        }

        let mut body = String::new();
        file.read_to_string(&mut body)?;

        Ok(EmailTemplate::new(subject, &body))
    }

    pub fn send(&self, mailer: &Mailer, params: &HashMap<&str, String>) -> EmptyResult {
        let (subject, body) = self.render(params)?;
        mailer.send(&subject, &body)
    }

    pub fn render(&self, params: &HashMap<&str, String>) -> GenericResult<(String, String)> {
        Ok((
            render_template(&self.subject, params)?,
            render_template(&self.body, params)?,
        ))
    }
}

fn render_template(template: &str, params: &HashMap<&str, String>) -> GenericResult<String> {
    let mut result = s!(template);

    // TODO: Use very naive implementation now because Rust doesn't have any mature template engine yet.
    for (key, value) in params {
        let key = s!("{{") + key + "}}";
        result = result.replace(&key, value);
    }

    Ok(result)
}
