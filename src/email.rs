use regex::Regex;
use libemail::Mailbox;

use lettre::email::EmailBuilder;
use lettre::mailer::Mailer as LettreMailer;
use lettre::transport::smtp::SmtpTransportBuilder;

use common::GenericResult;

pub struct Mailer {
    from: Mailbox,
    to: Mailbox,
}

impl Mailer {
    pub fn new(from: &str, to: &str) -> GenericResult<Mailer> {
        Ok(Mailer {
            from: try!(parse_email(from)),
            to: try!(parse_email(to)),
        })
    }

    pub fn send(&self, subject: &str, body: &str) -> GenericResult<()> {
        let email = try!(EmailBuilder::new()
            .to(self.to.clone())
            .from(self.from.clone())
            .subject(subject)
            .body(body)
            .build());

        let transport = try!(SmtpTransportBuilder::localhost()).build();

        try!(LettreMailer::new(transport).send(email));

        Ok(())
    }
}

fn parse_email(email: &str) -> GenericResult<Mailbox> {
    let email_address_re = r"(?P<address>[a-zA-Z0-9_.+-]+@[a-zA-Z0-9-]+\.[a-zA-Z0-9-.]+)";
    let email_re = Regex::new(&(s!("^") + email_address_re + "$")).unwrap();
    let email_with_name_re = Regex::new(&(s!(r"(?P<name>[^<]+)<") + email_address_re + ">$")).unwrap();

    Ok(match email_with_name_re.captures(email.trim()) {
        Some(captures) => Mailbox::new_with_name(
            s!(captures.name("name").unwrap().trim()), s!(captures.name("address").unwrap())),

        None => match email_re.captures(email) {
            Some(captures) => Mailbox::new(s!(captures.name("address").unwrap())),
            None => return Err!("Invalid email: '{}'", email)
        }
    })
}
