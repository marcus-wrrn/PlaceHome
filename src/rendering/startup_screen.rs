use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Style},
};

use crate::services::ServiceId;
use crate::supervisor::ServiceStatus;

pub struct StartupScreen;

impl StartupScreen {
    pub fn new() -> Self {
        Self
    }

    pub fn render(
        &self,
        area: Rect,
        buf: &mut Buffer,
        statuses: &HashMap<ServiceId, ServiceStatus>,
    ) {
        let vertical_sections = Layout::vertical([
            Constraint::Length(1),  // Top padding
            Constraint::Length(5),  // Title banner
            Constraint::Length(1),  // Spacing
            Constraint::Min(0),    // Service status + commands (fills remaining)
            Constraint::Length(1),  // Bottom padding
        ])
        .flex(Flex::Center)
        .split(area);

        self.render_title(buf, vertical_sections[1]);

        let content_sections = Layout::vertical([
            Constraint::Length(2 + statuses.len() as u16 + 1), // Services box
            Constraint::Length(1),                               // Spacing
            Constraint::Length(8),                               // Commands box
        ])
        .flex(Flex::Start)
        .split(vertical_sections[3]);

        self.render_service_statuses(buf, content_sections[0], statuses);
        self.render_commands(buf, content_sections[2]);
    }

    fn render_title(&self, buf: &mut Buffer, area: Rect) {
        let banner = [
            "╔══════════════════════════════════════╗",
            "║         PlaceNet Home Server         ║",
            "║        ─────────────────────         ║",
            "║          Process Supervisor          ║",
            "╚══════════════════════════════════════╝",
        ];

        for (i, line) in banner.iter().enumerate() {
            let y = area.y + i as u16;
            if y >= area.y + area.height {
                break;
            }
            self.render_centered_text(buf, area, y, line, Style::default().fg(Color::Cyan));
        }
    }

    fn render_service_statuses(
        &self,
        buf: &mut Buffer,
        area: Rect,
        statuses: &HashMap<ServiceId, ServiceStatus>,
    ) {
        let header = " Services ";
        let header_style = Style::default().fg(Color::Yellow);
        self.render_centered_text(buf, area, area.y, header, header_style);

        let border_y = area.y + 1;
        let border_str = "─".repeat(36);
        self.render_centered_text(
            buf,
            area,
            border_y,
            &border_str,
            Style::default().fg(Color::DarkGray),
        );

        let mut sorted_services: Vec<_> = statuses.iter().collect();
        sorted_services.sort_by_key(|(id, _)| format!("{:?}", id));

        for (i, (id, status)) in sorted_services.iter().enumerate() {
            let y = border_y + 1 + i as u16;
            if y >= area.y + area.height {
                break;
            }

            let name = format!("{:?}", id);
            let (status_text, status_color) = Self::format_status(status);
            let line = format!("  {:.<20} {}", name, status_text);

            let dots_end = 22; // "  " + name padded to 20 with dots
            let x = Self::centered_x(area, line.len());

            buf.set_string(
                x,
                y,
                &line[..dots_end.min(line.len())],
                Style::default().fg(Color::White),
            );
            // Render the status portion in its color
            if line.len() > dots_end {
                buf.set_string(
                    x + dots_end as u16,
                    y,
                    &line[dots_end..],
                    Style::default().fg(status_color),
                );
            }
        }
    }

    fn render_commands(&self, buf: &mut Buffer, area: Rect) {
        let header = " Commands ";
        let header_style = Style::default().fg(Color::Yellow);
        self.render_centered_text(buf, area, area.y, header, header_style);

        let border_y = area.y + 1;
        let border_str = "─".repeat(36);
        self.render_centered_text(
            buf,
            area,
            border_y,
            &border_str,
            Style::default().fg(Color::DarkGray),
        );

        let commands = [
            ("start", "Start all services"),
            ("stop", "Stop all services"),
            ("pause", "Pause service polling"),
            ("uninstall", "Remove and clean up"),
            ("q / Ctrl+C", "Quit"),
        ];

        for (i, (key, desc)) in commands.iter().enumerate() {
            let y = border_y + 1 + i as u16;
            if y >= area.y + area.height {
                break;
            }

            let key_str = format!("  {:<14}", key);
            let x = Self::centered_x(area, key_str.len() + desc.len());

            buf.set_string(x, y, &key_str, Style::default().fg(Color::Green));
            buf.set_string(
                x + key_str.len() as u16,
                y,
                desc,
                Style::default().fg(Color::Gray),
            );
        }
    }

    fn format_status(status: &ServiceStatus) -> (&str, Color) {
        match status {
            ServiceStatus::Running { .. } => ("RUNNING", Color::Green),
            ServiceStatus::Stopped => ("STOPPED", Color::DarkGray),
            ServiceStatus::Starting => ("STARTING", Color::Yellow),
            ServiceStatus::Unavailable => ("UNAVAILABLE", Color::Red),
            ServiceStatus::Failed { .. } => ("FAILED", Color::Red),
        }
    }

    fn render_centered_text(
        &self,
        buf: &mut Buffer,
        area: Rect,
        y: u16,
        text: &str,
        style: Style,
    ) {
        let x = Self::centered_x(area, text.len());
        buf.set_string(x, y, text, style);
    }

    fn centered_x(area: Rect, text_len: usize) -> u16 {
        if area.width >= text_len as u16 {
            area.x + (area.width - text_len as u16) / 2
        } else {
            area.x
        }
    }
}
