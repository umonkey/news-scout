# News Scout

This directory contains the code for a tool that monitors a part of the user's Telegram subscription, detects messages on a certain topic, and forwards them to the headquarters chat.

## Scope and Tech Stack

- Language: Rust.
- Database: SQLite.
- Interface: command line (daemon).

## File and Folder Structure

- `src/infra`: this contains connectors to external resources, like third party services or the system environment.
- `src/domain`: this contains business domain models, repositories and services that operate on them.
- `src/services`: this contains services that orchestrate domains and infra but don't contain their own domain data.

Model structure:

- `models.rs`: contains the structure that defines DTO, typically named after the domain, e.g. "Model".
- `repository.rs`: contains the database interface, e.g. "ModelRepository", which receives a db connection, and exposes methods like get, put, find_something, etc.

## Development Workflow and Git Policy

- After changing the code, run "make format check" to perform static analysis and run unit tests.
