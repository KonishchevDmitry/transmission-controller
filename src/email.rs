use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, BufReader, BufRead};
use std::path::Path;

use regex::Regex;
use libemail::Mailbox;
use log::debug;

use lettre::Transport;
use lettre::smtp::SmtpClient;
use lettre_email::EmailBuilder;

use common::{EmptyResult, GenericResult};

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
            from: parse_email_address(from)?,
            to: parse_email_address(to)?,
        })
    }

    pub fn send(&self, subject: &str, body: &str) -> EmptyResult {
        let mut builder = EmailBuilder::new();

        if let Some(ref name) = self.to.name {
            builder = builder.to((&self.to.address as &str, name as &str));
        } else {
            builder = builder.to(&self.to.address as &str);
        }

        if let Some(ref name) = self.from.name {
            builder = builder.from((&self.from.address as &str, name as &str));
        } else {
            builder = builder.from(&self.from.address as &str);
        }

        let email = builder.subject(subject).body(body).build().map_err(|e| format!(
            "Failed to construct a email: {}", e))?;

        debug!("Sending {:?} email to {}...", subject, self.to.address);
        let mut mailer = SmtpClient::new_unencrypted_localhost()?.transport();
        mailer.send(email.into())?;
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

fn parse_email_address(email: &str) -> GenericResult<Mailbox> {
    let email_address_re = r"(?P<address>[a-zA-Z0-9_.+-]+@[a-zA-Z0-9-]+\.[a-zA-Z0-9-.]+)";
    let email_re = Regex::new(&(s!("^") + email_address_re + "$")).unwrap();
    let email_with_name_re = Regex::new(&(s!(r"^(?P<name>[^<]+?)\s*<") + email_address_re + ">$")).unwrap();

    Ok(match email_with_name_re.captures(email.trim()) {
        Some(captures) => Mailbox::new_with_name(
            s!(captures.name("name").unwrap().as_str()), s!(captures.name("address").unwrap().as_str())),

        None => match email_re.captures(email) {
            Some(captures) => Mailbox::new(s!(captures.name("address").unwrap().as_str())),
            None => return Err!("Invalid email: '{}'", email)
        }
    })
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
