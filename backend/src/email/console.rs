use async_trait::async_trait;

use super::{MailError, Mailer};

#[derive(Clone, Default)]
pub struct ConsoleMailer;

#[async_trait]
impl Mailer for ConsoleMailer {
    async fn send(
        &self,
        sender_name: &str,
        sender_email: &str,
        to_email: &str,
        to_name: &str,
        subject: &str,
        html: &str,
    ) -> Result<(), MailError> {
        tracing::info!(
            to = %to_email,
            to_name = %to_name,
            from = %sender_email,
            from_name = %sender_name,
            subject = %subject,
            html_len = html.len(),
            "console email (not sent externally)"
        );
        Ok(())
    }
}
