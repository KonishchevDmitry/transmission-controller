use lettre::email::EmailBuilder;
use lettre::mailer::Mailer as LettreMailer;
use lettre::transport::smtp::SmtpTransportBuilder;

use common::GenericResult;

pub struct Mailer {
    from: String,
    to: String,
}

impl Mailer {
    pub fn new(from: String, to: String) -> Mailer {
        Mailer {from: from, to: to }
    }

    pub fn send(&self, subject: &str, body: &str) -> GenericResult<()> {
        let email = try!(EmailBuilder::new()
            .to(&self.to as &str)
            .from(&self.from as &str)
            .subject(subject)
            .body(body)
            .build());

        let transport = try!(SmtpTransportBuilder::localhost()).build();

        try!(LettreMailer::new(transport).send(email));

        Ok(())
    }
}
