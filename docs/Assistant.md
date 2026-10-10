# Personal Assistant

This service is an experimental (WIP) agent that helps me boost my productivity wrt Kanach Yerevan initiative. The goal is to automate routines which are resource consuming but valuable, and for which we don't have any volunteers. It is also a way to learn building agentic systems.

## The architecture

Currently it's just a pipeline where LLM is just a single fixed step in a hard-coded workflow.

The goal is to make it a truly agentic assistant, driven by a prompt, which was provided some tools to interact with the world (read and write telegram messages, read the calendar, report bugs, etc). This would help add non-linera logic easily.

Potential frameworks:

- [rig-core](https://crates.io/crates/rig-core)
