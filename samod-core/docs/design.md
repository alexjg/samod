# samod-core Design Document

## Overview

`samod-core` is a sans-IO implementation of the core logic for synchronizing Automerge documents across networks and storage. The library serves as a bridge between the pure in-memory Automerge CRDT library and real-world network connections and storage systems. It follows an event-driven architecture where all interactions happen through a central event loop that processes commands and manages IO operations asynchronously.

## Purpose and Goals

The primary purpose of `samod-core` is to synchronize Automerge documents between peers while maintaining compatibility with the JavaScript `automerge-repo` library. Key goals include:

- **Automerge Integration**: Wire up Automerge's sync protocol to network connections and persistent storage
- **Cross-Platform Compatibility**: Sans-IO design enables use across different async runtimes and FFI boundaries
- **Protocol Compatibility**: Implement the same wire protocol as JavaScript `automerge-repo` for interoperability
