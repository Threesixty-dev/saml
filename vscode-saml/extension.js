const vscode = require('vscode');
const cp = require('child_process');
const fs = require('fs');
const path = require('path');

function asPosition(value) {
  return new vscode.Position(value.line, value.character);
}

function asRange(value) {
  return new vscode.Range(asPosition(value.start), asPosition(value.end));
}

class Client {
  constructor(process, output, diagnostics) {
    this.process = process;
    this.output = output;
    this.diagnostics = diagnostics;
    this.nextId = 1;
    this.pending = new Map();
    this.buffer = Buffer.alloc(0);
    process.stdout.on('data', chunk => this.read(chunk));
    process.stderr.on('data', chunk => output.append(chunk.toString()));
    process.on('error', error => this.fail(error));
    process.on('exit', code => this.fail(new Error(`SAML server exited (${code})`)));
  }

  fail(error) {
    this.output.appendLine(error.message);
    for (const pending of this.pending.values()) pending.reject(error);
    this.pending.clear();
  }

  send(message) {
    const body = Buffer.from(JSON.stringify({ jsonrpc: '2.0', ...message }), 'utf8');
    this.process.stdin.write(`Content-Length: ${body.length}\r\n\r\n`);
    this.process.stdin.write(body);
  }

  request(method, params) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({ id, method, params });
    });
  }

  notify(method, params) {
    this.send({ method, params });
  }

  read(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    while (true) {
      const headerEnd = this.buffer.indexOf('\r\n\r\n');
      if (headerEnd < 0) return;
      const header = this.buffer.subarray(0, headerEnd).toString('ascii');
      const match = /^Content-Length:\s*(\d+)\s*$/im.exec(header);
      if (!match) {
        this.fail(new Error('Invalid response from SAML server'));
        this.buffer = Buffer.alloc(0);
        return;
      }
      const length = Number(match[1]);
      const bodyStart = headerEnd + 4;
      if (this.buffer.length < bodyStart + length) return;
      const body = this.buffer.subarray(bodyStart, bodyStart + length);
      this.buffer = this.buffer.subarray(bodyStart + length);
      try {
        this.handle(JSON.parse(body.toString('utf8')));
      } catch (error) {
        this.output.appendLine(`SAML client: ${error.message}`);
      }
    }
  }

  handle(message) {
    if (message.id !== undefined) {
      const pending = this.pending.get(message.id);
      if (pending) {
        this.pending.delete(message.id);
        if (message.error) pending.reject(new Error(message.error.message));
        else pending.resolve(message.result);
      }
      return;
    }
    if (message.method === 'textDocument/publishDiagnostics') {
      const { uri, diagnostics } = message.params;
      this.diagnostics.set(vscode.Uri.parse(uri), diagnostics.map(item => {
        const diagnostic = new vscode.Diagnostic(asRange(item.range), item.message,
          (item.severity || 1) - 1);
        diagnostic.source = item.source;
        return diagnostic;
      }));
    }
  }
}

async function activate(context) {
  const folder = vscode.workspace.workspaceFolders?.[0];
  if (!folder) return;
  const configured = vscode.workspace.getConfiguration('saml').get('serverPath');
  const binary = configured || path.join(folder.uri.fsPath, 'target', 'debug', 'saml-lsp');
  if (!fs.existsSync(binary)) {
    vscode.window.showErrorMessage(`SAML server not found: ${binary}. Run cargo build in the workspace.`);
    return;
  }

  const output = vscode.window.createOutputChannel('SAML Language Server');
  const diagnostics = vscode.languages.createDiagnosticCollection('saml');
  const process = cp.spawn(binary, [], { cwd: folder.uri.fsPath, stdio: 'pipe' });
  const client = new Client(process, output, diagnostics);
  context.subscriptions.push(output, diagnostics, { dispose: () => process.kill() });

  try {
    await client.request('initialize', {
      processId: process.pid,
      rootUri: folder.uri.toString(),
      workspaceFolders: vscode.workspace.workspaceFolders.map(item => ({
        uri: item.uri.toString(), name: item.name
      })),
      capabilities: {}
    });
    client.notify('initialized', {});
  } catch (error) {
    vscode.window.showErrorMessage(`SAML server failed to initialize: ${error.message}`);
    return;
  }

  const selector = { language: 'saml', scheme: 'file' };
  const open = document => {
    if (document.languageId === 'saml') {
      client.notify('textDocument/didOpen', { textDocument: {
        uri: document.uri.toString(), languageId: 'saml', version: document.version,
        text: document.getText()
      }});
    }
  };
  for (const document of vscode.workspace.textDocuments) open(document);
  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument(open),
    vscode.workspace.onDidChangeTextDocument(event => {
      if (event.document.languageId === 'saml') {
        client.notify('textDocument/didChange', {
          textDocument: { uri: event.document.uri.toString(), version: event.document.version },
          contentChanges: [{ text: event.document.getText() }]
        });
      }
    }),
    vscode.workspace.onDidCloseTextDocument(document => {
      if (document.languageId === 'saml') {
        client.notify('textDocument/didClose', { textDocument: { uri: document.uri.toString() } });
      }
    }),
    vscode.languages.registerDocumentSymbolProvider(selector, {
      async provideDocumentSymbols(document) {
        const items = await client.request('textDocument/documentSymbol', {
          textDocument: { uri: document.uri.toString() }
        });
        const convert = item => {
          const symbol = new vscode.DocumentSymbol(item.name, item.detail || '', item.kind - 1,
            asRange(item.range), asRange(item.selectionRange));
          symbol.children = (item.children || []).map(convert);
          return symbol;
        };
        return items.map(convert);
      }
    }),
    vscode.languages.registerDefinitionProvider(selector, {
      async provideDefinition(document, position) {
        const locations = await client.request('textDocument/definition', {
          textDocument: { uri: document.uri.toString() }, position
        });
        return locations.map(item => new vscode.Location(vscode.Uri.parse(item.uri), asRange(item.range)));
      }
    }),
    vscode.languages.registerHoverProvider(selector, {
      async provideHover(document, position) {
        const result = await client.request('textDocument/hover', {
          textDocument: { uri: document.uri.toString() }, position
        });
        return result ? new vscode.Hover(new vscode.MarkdownString(result.contents.value)) : null;
      }
    }),
    vscode.languages.registerCompletionItemProvider(selector, {
      async provideCompletionItems(document, position) {
        const items = await client.request('textDocument/completion', {
          textDocument: { uri: document.uri.toString() }, position
        });
        return items.map(item => new vscode.CompletionItem(item.label, item.kind - 1));
      }
    })
  );
  output.appendLine(`SAML server running: ${binary}`);
}

function deactivate() {}

module.exports = { activate, deactivate };
