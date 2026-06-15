use lettre::{
    message::header::ContentType, transport::smtp::authentication::Credentials,
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};

use cfg::model::EmailConfig;

/// Fire-and-forget password-reset email via the configured SMTP relay.
/// If SMTP_HOST is not set, prints the code to the log (dev fallback).
pub fn send_reset_code(cfg: &EmailConfig, to_email: &str, code: &str) {
    if cfg.smtp_host.is_empty() {
        tracing::warn!(
            email = to_email,
            code,
            "SMTP not configured — password reset code printed here for dev use"
        );
        return;
    }

    let host = cfg.smtp_host.clone();
    let port = cfg.smtp_port;
    let user = cfg.smtp_user.clone();
    let pass = cfg.smtp_password.clone();
    let from = cfg.from_address.clone();
    let to = to_email.to_string();
    let code = code.to_string();

    tokio::spawn(async move {
        let from_addr = match from.parse() {
            Ok(a) => a,
            Err(e) => {
                tracing::error!(error = %e, from, "invalid SMTP_FROM address");
                return;
            }
        };
        let to_addr = match to.parse() {
            Ok(a) => a,
            Err(e) => {
                tracing::error!(error = %e, to, "invalid recipient address");
                return;
            }
        };

        let email = match Message::builder()
            .from(from_addr)
            .to(to_addr)
            .subject("Your TradingBot password reset code")
            .header(ContentType::TEXT_PLAIN)
            .body(format!(
                "Your password reset code is:\n\n    {code}\n\nIt expires in 15 minutes.\n\nIf you didn't request this, ignore this email.\n"
            )) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(error = %e, "failed to build email");
                return;
            }
        };

        let creds = Credentials::new(user, pass);
        let transport = match AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host) {
            Ok(b) => b.port(port).credentials(creds).build(),
            Err(e) => {
                tracing::error!(error = %e, "failed to build SMTP transport");
                return;
            }
        };

        match transport.send(email).await {
            Ok(_) => tracing::info!(relay = %host, recipient = %to, "password reset email sent"),
            Err(e) => tracing::error!(error = %e, relay = %host, "failed to send email"),
        }
    });
}
