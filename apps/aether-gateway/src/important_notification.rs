use crate::admin_api::AdminAppState;
use crate::email_delivery::{
    read_smtp_delivery_config, send_smtp_email, ComposedEmail, SmtpDeliveryConfig,
};
use crate::handlers::shared::{
    decrypt_catalog_secret_with_fallbacks, system_config_bool, system_config_string,
};
use crate::{AppState, GatewayError};
use axum::body::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::warn;

pub(crate) const IMPORTANT_NOTIFICATION_ENABLED_KEY: &str = "module.important_notification.enabled";
pub(crate) const LEGACY_NOTIFICATION_EMAIL_ENABLED_KEY: &str = "module.notification_email.enabled";
pub(crate) const IMPORTANT_NOTIFICATION_EMAIL_ENABLED_KEY: &str =
    "module.important_notification.email_enabled";
pub(crate) const IMPORTANT_NOTIFICATION_EMAIL_RECIPIENTS_KEY: &str =
    "module.important_notification.email_recipients";
pub(crate) const IMPORTANT_NOTIFICATION_SERVER_CHAN_ENABLED_KEY: &str =
    "module.important_notification.server_chan_enabled";
pub(crate) const IMPORTANT_NOTIFICATION_SERVER_CHAN_SEND_KEY_KEY: &str =
    "module.important_notification.server_chan_send_key";
pub(crate) const IMPORTANT_NOTIFICATION_SERVER_CHAN_TEMPLATE_KEY: &str =
    "module.important_notification.server_chan_template";

const SERVER_CHAN_API_BASE: &str = "https://sctapi.ftqq.com";

#[derive(Debug, Clone)]
pub(crate) struct ImportantNotification {
    pub(crate) title: String,
    pub(crate) markdown_body: String,
    pub(crate) text_body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportantNotificationChannelFilter {
    All,
    Email,
    ServerChan,
}

#[derive(Debug, Clone)]
struct ImportantNotificationConfig {
    module_enabled: bool,
    email_enabled: bool,
    email_recipients: Vec<String>,
    server_chan_enabled: bool,
    server_chan_send_key: Option<String>,
    server_chan_template: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ImportantNotificationChannelReport {
    pub(crate) channel: &'static str,
    pub(crate) success: bool,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ImportantNotificationDeliveryReport {
    pub(crate) success: bool,
    pub(crate) channels: Vec<ImportantNotificationChannelReport>,
}

#[derive(Debug, Deserialize)]
struct ImportantNotificationTestRequest {
    #[serde(default)]
    channel: Option<String>,
}

pub(crate) async fn important_notification_module_enabled(
    state: &AppState,
) -> Result<bool, GatewayError> {
    let canonical = state
        .read_system_config_json_value(IMPORTANT_NOTIFICATION_ENABLED_KEY)
        .await?;
    if canonical.is_some() {
        return Ok(system_config_bool(canonical.as_ref(), false));
    }
    let legacy = state
        .read_system_config_json_value(LEGACY_NOTIFICATION_EMAIL_ENABLED_KEY)
        .await?;
    Ok(system_config_bool(legacy.as_ref(), false))
}

pub(crate) async fn important_notification_configured(
    state: &AppState,
) -> Result<bool, GatewayError> {
    let config = read_important_notification_config(state).await?;
    let smtp_config = read_smtp_delivery_config(state).await?;
    Ok(
        (config.email_enabled && !config.email_recipients.is_empty() && smtp_config.is_some())
            || (config.server_chan_enabled && config.server_chan_send_key.is_some()),
    )
}

pub(crate) async fn send_important_notification(
    state: &AppState,
    notification: ImportantNotification,
) -> Result<ImportantNotificationDeliveryReport, GatewayError> {
    send_important_notification_with_filter(
        state,
        notification,
        ImportantNotificationChannelFilter::All,
    )
    .await
}

pub(crate) async fn send_important_notification_with_filter(
    state: &AppState,
    notification: ImportantNotification,
    channel_filter: ImportantNotificationChannelFilter,
) -> Result<ImportantNotificationDeliveryReport, GatewayError> {
    dispatch_important_notification(state, notification, channel_filter, false).await
}

async fn dispatch_important_notification(
    state: &AppState,
    notification: ImportantNotification,
    channel_filter: ImportantNotificationChannelFilter,
    bypass_enable_checks: bool,
) -> Result<ImportantNotificationDeliveryReport, GatewayError> {
    let config = read_important_notification_config(state).await?;
    if !bypass_enable_checks && !config.module_enabled {
        return Ok(ImportantNotificationDeliveryReport {
            success: false,
            channels: vec![ImportantNotificationChannelReport {
                channel: "module",
                success: false,
                message: "重要通知模块未启用".to_string(),
            }],
        });
    }

    let mut reports = Vec::new();
    if matches!(
        channel_filter,
        ImportantNotificationChannelFilter::All | ImportantNotificationChannelFilter::Email
    ) {
        maybe_send_email_notification(
            state,
            &config,
            &notification,
            bypass_enable_checks,
            &mut reports,
        )
        .await;
    }

    if matches!(
        channel_filter,
        ImportantNotificationChannelFilter::All | ImportantNotificationChannelFilter::ServerChan
    ) {
        maybe_send_server_chan_notification(
            state,
            &config,
            &notification,
            bypass_enable_checks,
            &mut reports,
        )
        .await;
    }

    if reports.is_empty() {
        reports.push(ImportantNotificationChannelReport {
            channel: "none",
            success: false,
            message: "未启用可用的通知通道".to_string(),
        });
    }
    let success = reports.iter().any(|report| report.success);
    Ok(ImportantNotificationDeliveryReport {
        success,
        channels: reports,
    })
}

pub(crate) async fn build_important_notification_test_payload(
    state: &AdminAppState<'_>,
    request_body: Option<&Bytes>,
) -> Result<Value, GatewayError> {
    let request = match request_body.filter(|body| !body.is_empty()) {
        Some(body) => serde_json::from_slice::<ImportantNotificationTestRequest>(body)
            .unwrap_or(ImportantNotificationTestRequest { channel: None }),
        None => ImportantNotificationTestRequest { channel: None },
    };
    let filter = match request
        .channel
        .as_deref()
        .map(str::trim)
        .unwrap_or("all")
        .to_ascii_lowercase()
        .as_str()
    {
        "email" => ImportantNotificationChannelFilter::Email,
        "server_chan" | "serverchan" | "serve_chan" => {
            ImportantNotificationChannelFilter::ServerChan
        }
        _ => ImportantNotificationChannelFilter::All,
    };
    let report = dispatch_important_notification(
        state.app(),
        ImportantNotification {
            title: "Aether 重要通知测试".to_string(),
            markdown_body: "这是一条来自 Aether 的重要通知测试。".to_string(),
            text_body: "这是一条来自 Aether 的重要通知测试。".to_string(),
        },
        filter,
        true,
    )
    .await?;

    Ok(json!({
        "success": report.success,
        "message": if report.success { "测试通知已发送" } else { "测试通知发送失败" },
        "channels": report.channels,
    }))
}

async fn read_important_notification_config(
    state: &AppState,
) -> Result<ImportantNotificationConfig, GatewayError> {
    let module_enabled = important_notification_module_enabled(state).await?;
    let email_enabled = state
        .read_system_config_json_value(IMPORTANT_NOTIFICATION_EMAIL_ENABLED_KEY)
        .await?;
    let email_recipients = state
        .read_system_config_json_value(IMPORTANT_NOTIFICATION_EMAIL_RECIPIENTS_KEY)
        .await?;
    let server_chan_enabled = state
        .read_system_config_json_value(IMPORTANT_NOTIFICATION_SERVER_CHAN_ENABLED_KEY)
        .await?;
    let server_chan_send_key = state
        .read_system_config_json_value(IMPORTANT_NOTIFICATION_SERVER_CHAN_SEND_KEY_KEY)
        .await?;
    let server_chan_template = state
        .read_system_config_json_value(IMPORTANT_NOTIFICATION_SERVER_CHAN_TEMPLATE_KEY)
        .await?;

    Ok(ImportantNotificationConfig {
        module_enabled,
        email_enabled: system_config_bool(email_enabled.as_ref(), false),
        email_recipients: parse_recipient_list(email_recipients.as_ref()),
        server_chan_enabled: system_config_bool(server_chan_enabled.as_ref(), false),
        server_chan_send_key: system_config_string(server_chan_send_key.as_ref()).map(|value| {
            decrypt_catalog_secret_with_fallbacks(state.encryption_key(), &value).unwrap_or(value)
        }),
        server_chan_template: system_config_string(server_chan_template.as_ref()),
    })
}

async fn maybe_send_email_notification(
    state: &AppState,
    config: &ImportantNotificationConfig,
    notification: &ImportantNotification,
    bypass_channel_toggle: bool,
    reports: &mut Vec<ImportantNotificationChannelReport>,
) {
    if !bypass_channel_toggle && !config.email_enabled {
        return;
    }
    if config.email_recipients.is_empty() {
        reports.push(ImportantNotificationChannelReport {
            channel: "email",
            success: false,
            message: "未配置邮件收件人".to_string(),
        });
        return;
    }
    let smtp_config = match read_smtp_delivery_config(state).await {
        Ok(Some(config)) => config,
        Ok(None) => {
            reports.push(ImportantNotificationChannelReport {
                channel: "email",
                success: false,
                message: "SMTP 配置不完整".to_string(),
            });
            return;
        }
        Err(err) => {
            reports.push(ImportantNotificationChannelReport {
                channel: "email",
                success: false,
                message: format!("读取 SMTP 配置失败: {err:?}"),
            });
            return;
        }
    };

    let mut sent = 0usize;
    let mut failed = 0usize;
    for recipient in &config.email_recipients {
        match send_single_email_notification(smtp_config.clone(), recipient, notification).await {
            Ok(()) => sent += 1,
            Err(err) => {
                failed += 1;
                warn!(
                    error = ?err,
                    recipient = %recipient,
                    "failed to send important notification email"
                );
            }
        }
    }

    reports.push(ImportantNotificationChannelReport {
        channel: "email",
        success: sent > 0,
        message: if failed == 0 {
            format!("邮件通知已发送给 {sent} 个收件人")
        } else {
            format!("邮件通知成功 {sent} 个，失败 {failed} 个")
        },
    });
}

async fn send_single_email_notification(
    smtp_config: SmtpDeliveryConfig,
    recipient: &str,
    notification: &ImportantNotification,
) -> Result<(), GatewayError> {
    send_smtp_email(
        smtp_config,
        ComposedEmail {
            to_email: recipient.to_string(),
            subject: notification.title.clone(),
            html_body: build_notification_html(notification),
            text_body: notification.text_body.clone(),
        },
    )
    .await
}

async fn maybe_send_server_chan_notification(
    state: &AppState,
    config: &ImportantNotificationConfig,
    notification: &ImportantNotification,
    bypass_channel_toggle: bool,
    reports: &mut Vec<ImportantNotificationChannelReport>,
) {
    if !bypass_channel_toggle && !config.server_chan_enabled {
        return;
    }
    let Some(send_key) = config.server_chan_send_key.as_deref() else {
        reports.push(ImportantNotificationChannelReport {
            channel: "server_chan",
            success: false,
            message: "未配置 Server 酱 SendKey".to_string(),
        });
        return;
    };
    match send_server_chan_notification(
        state,
        send_key,
        config.server_chan_template.as_deref(),
        notification,
    )
    .await
    {
        Ok(()) => reports.push(ImportantNotificationChannelReport {
            channel: "server_chan",
            success: true,
            message: "Server 酱通知已发送".to_string(),
        }),
        Err(err) => {
            warn!(error = ?err, "failed to send server chan important notification");
            reports.push(ImportantNotificationChannelReport {
                channel: "server_chan",
                success: false,
                message: format!("Server 酱通知发送失败: {err:?}"),
            });
        }
    }
}

async fn send_server_chan_notification(
    state: &AppState,
    send_key: &str,
    template: Option<&str>,
    notification: &ImportantNotification,
) -> Result<(), GatewayError> {
    let send_key = send_key.trim();
    if send_key.is_empty() {
        return Err(GatewayError::Internal(
            "Server 酱 SendKey 不能为空".to_string(),
        ));
    }
    let desp = render_server_chan_desp(template, notification);
    let url = format!("{SERVER_CHAN_API_BASE}/{send_key}.send");
    let response = state
        .client
        .post(url)
        .form(&[
            ("title", notification.title.as_str()),
            ("desp", desp.as_str()),
        ])
        .send()
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
    if !status.is_success() {
        return Err(GatewayError::Internal(format!(
            "Server 酱返回 HTTP {status}: {text}"
        )));
    }
    if let Ok(payload) = serde_json::from_str::<Value>(&text) {
        let code_is_ok = payload
            .get("code")
            .and_then(|value| {
                value
                    .as_i64()
                    .map(|code| code == 0)
                    .or_else(|| value.as_str().map(|code| code.trim() == "0"))
            })
            .unwrap_or(true);
        if !code_is_ok {
            return Err(GatewayError::Internal(format!(
                "Server 酱返回失败: {payload}"
            )));
        }
    }
    Ok(())
}

fn render_server_chan_desp(template: Option<&str>, notification: &ImportantNotification) -> String {
    match template {
        Some(template) if !template.trim().is_empty() => template
            .replace("{title}", &notification.title)
            .replace("{body}", &notification.markdown_body),
        _ => notification.markdown_body.clone(),
    }
}

fn parse_recipient_list(value: Option<&Value>) -> Vec<String> {
    let mut recipients = Vec::new();
    match value {
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(raw) = item.as_str() {
                    push_recipient_parts(&mut recipients, raw);
                }
            }
        }
        Some(Value::String(raw)) => push_recipient_parts(&mut recipients, raw),
        _ => {}
    }
    recipients.sort();
    recipients.dedup();
    recipients
}

fn push_recipient_parts(recipients: &mut Vec<String>, raw: &str) {
    for item in raw
        .split([',', ';', '\n', '\r'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        recipients.push(item.to_string());
    }
}

fn build_notification_html(notification: &ImportantNotification) -> String {
    format!(
        "<!doctype html><html><body><h2>{}</h2><pre style=\"font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;white-space:pre-wrap;line-height:1.6\">{}</pre></body></html>",
        escape_html(&notification.title),
        escape_html(&notification.text_body),
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::{parse_recipient_list, render_server_chan_desp, ImportantNotification};
    use serde_json::json;

    #[test]
    fn parse_recipient_list_accepts_arrays_and_delimiters() {
        assert_eq!(
            parse_recipient_list(Some(&json!([
                "ops@example.com, admin@example.com",
                "ops@example.com"
            ]))),
            vec![
                "admin@example.com".to_string(),
                "ops@example.com".to_string()
            ]
        );
    }

    fn sample_notification() -> ImportantNotification {
        ImportantNotification {
            title: "告警".to_string(),
            markdown_body: "原始正文".to_string(),
            text_body: "原始正文".to_string(),
        }
    }

    #[test]
    fn server_chan_desp_uses_template_when_provided() {
        let rendered = render_server_chan_desp(
            Some("**{title}**\n\n{body}\n\n--end--"),
            &sample_notification(),
        );
        assert_eq!(rendered, "**告警**\n\n原始正文\n\n--end--");
    }

    #[test]
    fn server_chan_desp_falls_back_to_markdown_body_for_empty_template() {
        assert_eq!(
            render_server_chan_desp(None, &sample_notification()),
            "原始正文"
        );
        assert_eq!(
            render_server_chan_desp(Some("   "), &sample_notification()),
            "原始正文"
        );
    }
}
