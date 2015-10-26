pub struct Mailer {
    from: String,
    to: String,
}

impl Mailer {
    pub fn new(from: String, to: String) -> Mailer {
        Mailer {from: from, to: to }
    }

    pub fn send(&self, subject: &str, body: &str) {
        /*
        use lettre::transport::smtp::{SmtpTransport, SmtpTransportBuilder};
        use lettre::email::EmailBuilder;
        use lettre::transport::EmailTransport;
        use lettre::mailer::Mailer;

        // Create an email
        let email = EmailBuilder::new()
            // Addresses can be specified by the couple (email, alias)
            .to(("konishchev@gmail.com", "Тестовое имя"))
            .from("server@konishchev.ru")
            .subject("Hi, Hello world")
            .body("Hello world.")
            .build().unwrap();

        // Open a local connection on port 25
        let mut mailer =
        Mailer::new(SmtpTransportBuilder::localhost().unwrap().build());
        // Send the email
        let result = mailer.send(email);

        assert!(result.is_ok());
        */
    }
}
