## Goal

Build a lightweight desktop timer application for Linux (Xubuntu/X11) using Rust.

This application is for personal use only.

It must run entirely locally.

There is:

* no backend
* no frontend web application
* no REST API
* no cloud
* no authentication
* no synchronization

The application stores everything in a local SQLite database.

The executable should consume very little RAM and CPU.

The application should live in the system tray when run, but it must not start automatically with the desktop session.


---

# Technology Stack

Language

* Rust (stable)

Database

* SQLite

Recommended crates

* rusqlite
* chrono
* notify-rust
* tray-icon
* x11rb (or another suitable X11 crate for idle detection)
* anyhow
* tracing
* tracing-subscriber

Do not introduce unnecessary dependencies.

---

# Application Requirements

The application has exactly four timer operations.

* Start
* Pause
* Resume
* Stop

No additional timer features should be implemented unless requested.

---

# Timer State Machine

Idle

↓

Running

↓

Paused

↓

Running

↓

Stopped

Invalid transitions should be prevented.

Examples

Running -> Start

Paused -> Pause

Idle -> Resume

must all be rejected.

---

# Idle Detection

The application must monitor keyboard and mouse activity.

If there is no user input for 5 minutes:

Show a desktop notification.

Message:

"You have been idle for 5 minutes. Are you still working?"

Options

Continue

Pause Timer

If the user ignores the notification for 30 seconds:

Automatically pause the timer.

When user activity resumes:

Display another notification.

Message:

"Resume timer? (You were inactive for [duration])"


If confirmed:

Resume.

Otherwise remain paused.

The idle timeout should be configurable.

Default

5 minutes.

---

# System Tray

The application should live in the system tray.

Tray menu

Start

Pause

Resume

Stop

Exit

The tray icon should visually indicate timer state if practical.

The user must be able to see the timer (e.g., elapsed session time). This may be displayed dynamically in the system tray (e.g., icon, title, or tooltip) or in a separate simple UI.


---

# Database

Use SQLite.

Database location

~/.local/share/worktimer/worktimer.db

Create the database automatically if it does not exist.

Use migrations.

Schema

sessions

* id
* started_at
* ended_at
* created_at

events

* id
* session_id
* event_type
* created_at

Event types

START

PAUSE

RESUME

STOP

AUTO_PAUSE

No ORM.

Use plain SQL through rusqlite.

---

# Logging

Write logs to

~/.local/state/worktimer/app.log

Log

application start

application exit

timer actions

database errors

unexpected failures

---

# Configuration

Configuration file

~/.config/worktimer/config.toml

Default

idle_timeout_minutes = 5

auto_pause_after_notification_seconds = 30


---

# Notifications

Use Linux desktop notifications.

Do not create custom popup windows.

Notifications

Timer Started

Timer Paused

Timer Resumed

Timer Stopped

Idle Detected

Resume Timer

---

# Desktop Integration

Support launching the application from the desktop environment menu (e.g., 'All Applications') by installing a standard `.desktop` launcher file. Do not start the application automatically on login.


---

# Code Organization

src/

main.rs

timer.rs

database.rs

idle.rs

tray.rs

notifications.rs

config.rs

logging.rs

state.rs

Keep modules independent.

Separate business logic from platform-specific code.

---

# Error Handling

Never panic for expected failures.

Use Result.

Display user-friendly notifications for recoverable failures.

Log all unexpected errors.

---

# Performance

CPU usage should remain near zero while idle.

Memory usage should remain below approximately 20 MB during normal operation.

Avoid polling when event-based approaches are available.

---

# Future Features

Do not implement these now.

Daily reports

Weekly reports

CSV export

Tags

Projects

Tasks

Statistics

Charts

Screenshots

Keyboard logging

Mouse logging

Application monitoring

Website monitoring

Cloud sync

Multi-user support

Network communication

---

# Coding Standards

Use idiomatic Rust.

Keep functions small.

Document public functions.

Avoid global mutable state.

Avoid unnecessary abstractions.

Write maintainable code.

---

# Development Strategy

Implement in small milestones.

Milestone 1

SQLite

Timer state machine

CLI testing

Milestone 2

System tray

Milestone 3

Notifications

Milestone 4

Idle detection

Milestone 5

Desktop launcher integration (.desktop file)


Milestone 6

Polish and testing

After each milestone:

* ensure the project builds successfully
* ensure cargo fmt passes
* ensure cargo clippy passes without warnings where practical
* ensure cargo test passes

Do not begin the next milestone until the current milestone is complete and verified.
