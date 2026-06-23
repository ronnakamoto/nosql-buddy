# Product

## Register
product

## Users
NoSQLBuddy is for developers, database engineers, SREs, data platform teams, and technical operators who manage MongoDB databases across local, staging, and production environments. They are often comparing environments, diagnosing query behavior, editing documents, reviewing indexes, and moving cautiously around live data.

## Product Purpose
NoSQLBuddy is a cross-platform MongoDB management studio for connection management, data browsing, query execution, aggregation building, SQL-to-Mongo translation, index/schema operations, performance inspection, and controlled migration workflows. Success means users can inspect and change MongoDB data safely without reaching for mongosh for routine work, while every destructive action is explicit, typed, auditable, and reversible where feasible.

## Brand Personality
Precise, calm, operational. The interface should feel like a trusted instrument panel for production data work: dense enough for experts, clear enough to avoid accidental writes, and restrained enough to stay out of the workflow.

## Anti-references
Do not look like a Tauri starter app, generic SaaS dashboard, neon database toy, glassmorphism demo, gradient-heavy AI app, or marketing page. Avoid oversized empty cards, fake native controls, decorative motion, hidden destructive actions, and anything that makes production data operations feel casual.

## Design Principles
- Data safety is visible: writes show scope, affected counts, dry runs, confirmations, and clear failure states.
- Expert density beats decoration: show more relevant state without visual noise.
- Native behavior first: menus, dialogs, tray, windows, and shortcuts should follow the platform.
- Query work is inspectable: generated pipelines, explain plans, schema inference, and migration scripts are always reviewable before execution.
- Secrets stay secret: credentials never appear in logs, exports, history, screenshots, or routine IPC responses.

## Accessibility & Inclusion
Target WCAG 2.2 AA for product UI. Every primary action must be keyboard reachable, focus rings must be visible, body text contrast must meet at least 4.5:1, destructive actions must not rely on color alone, and all motion must respect prefers-reduced-motion.