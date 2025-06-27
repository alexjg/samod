import {
  Repo,
  isValidAutomergeUrl,
  parseAutomergeUrl,
} from "@automerge/automerge-repo";
import { BrowserWebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";
import { command, run, string, positional, number, subcommands } from "cmd-ts";
import { next as A } from "@automerge/automerge";

const create = command({
  name: "create",
  args: {
    port: positional({
      type: number,
      displayName: "port",
      description: "The port to connect to",
    }),
  },
  handler: ({ port }) => {
    const repo = new Repo({
      network: [new BrowserWebSocketClientAdapter(`ws://localhost:${port}`)],
    });
    const doc = repo.create<{ foo: string }>();
    doc.change((d) => (d.foo = "bar"));
    console.log(doc.url);
    console.log(A.getHeads(doc.doc()).join(","));
  },
});

const fetch = command({
  name: "fetch",
  args: {
    port: positional({
      type: number,
      displayName: "port",
      description: "The port to connect to",
    }),
    docUrl: positional({
      type: string,
      displayName: "docUrl",
      description: "The document url to fetch",
    }),
  },
  handler: ({ port, docUrl }) => {
    const repo = new Repo({
      network: [new BrowserWebSocketClientAdapter(`ws://localhost:${port}`)],
    });
    if (isValidAutomergeUrl(docUrl)) {
    } else {
      throw new Error("Invalid docUrl");
    }
    const doc = repo.find(docUrl);
    repo.find(docUrl).then((d) => console.log(A.getHeads(d.doc()).join(",")));
  },
});

const sendEphemeral = command({
  name: "send-ephemeral",
  args: {
    port: positional({
      type: number,
      displayName: "port",
      description: "The port to connect to",
    }),
    docUrl: positional({
      type: string,
      displayName: "docUrl",
      description: "The document url to fetch",
    }),
    message: positional({
      type: string,
      displayName: "message",
      description: "The message to send",
    }),
  },
  handler: ({ port, docUrl, message }) => {
    const repo = new Repo({
      network: [new BrowserWebSocketClientAdapter(`ws://localhost:${port}`)],
    });
    if (!isValidAutomergeUrl(docUrl)) {
      throw new Error("Invalid docUrl");
    }
    repo
      .find(docUrl)
      .then((doc) => {
        doc.broadcast({ message });
        process.exit(0);
      })
      .catch((e) => {
        console.error(e);
        process.exit(1);
      });
  },
});

const receiveEphemeral = command({
  name: "receive-ephemeral",
  args: {
    port: positional({
      type: number,
      displayName: "port",
      description: "The port to connect to",
    }),
    docUrl: positional({
      type: string,
      displayName: "docUrl",
      description: "The document url to fetch",
    }),
  },
  handler: ({ port, docUrl }) => {
    const repo = new Repo({
      network: [new BrowserWebSocketClientAdapter(`ws://localhost:${port}`)],
    });
    if (!isValidAutomergeUrl(docUrl)) {
      throw new Error("Invalid docUrl");
    }
    const doc = repo.find(docUrl);
    repo.find(docUrl).then((doc) => {
      doc.on("ephemeral-message", ({ message }) => {
        if (typeof message === "object" && "message" in message) {
          console.log(message.message);
        }
      });
    });
  },
});

const app = subcommands({
  name: "client",
  cmds: {
    create,
    fetch,
    "send-ephemeral": sendEphemeral,
    "receive-ephemeral": receiveEphemeral,
  },
});

run(app, process.argv.slice(2));
