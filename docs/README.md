# VilSend Documentation

This documentation describes the implementation in this repository as inspected on 2026-09-05. It distinguishes verified behavior from recommendations. The repository is a Tauri v2 desktop application with a React/Vite frontend and a Rust local transfer/WebSocket runtime.

## Index

- [Architecture](ARCHITECTURE.md)
- [Frontend](FRONTEND.md)
- [Tauri and Rust](TAURI.md)
- [Backend and local services](BACKEND.md)
- [API contracts](API.md)
- [Authentication and device identity](AUTHENTICATION.md)
- [File transfer](FILE_TRANSFER.md)
- [Security](SECURITY.md)
- [Performance](PERFORMANCE.md)
- [Development](DEVELOPMENT.md)
- [Build and release](BUILD_AND_RELEASE.md)
- [Decisions](DECISIONS.md)
- [Engineering review](ENGINEERING_REVIEW.md)
- [Improvement roadmap](IMPROVEMENT_ROADMAP.md)
- [Repository map](REPOSITORY_MAP.md)
- [Project structure](PROJECT_STRUCTURE.md)
- [Important flows](FLOWS.md)

## Scope and important absences

The repository contains no Spring Boot, Maven, Gradle, Dockerfile, database-server, or central backend source. Central API and WebSocket services are external dependencies configured through `VITE_API_BASE_URL`, `WS_URL`, `API_URL`, and production defaults. Their implementation, schema, authorization policy, and deployment are **not found in repository**.

The existing root README is a starter template and is not an operational guide. The Microsoft Store documents describe intended packaging, but the Store workflow is currently commented out in `.github/workflows/release.yml`.
