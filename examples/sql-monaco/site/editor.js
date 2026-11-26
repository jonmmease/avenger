// An editor of an avenger-sql project in the page: Monaco edits the project's files, and
// avenger-sql-web, which holds the project in memory, answers the editor's requests and builds the
// charts of the open chart script, which its `ChartView` draws. Positions cross between them as
// UTF-16 offsets, which Monaco's models convert to lines and columns.

const MONACO = "https://cdn.jsdelivr.net/npm/monaco-editor@0.52.2/min";

// Monaco's workers load from the CDN through a script of the page's origin.
window.MonacoEnvironment = {
  getWorkerUrl() {
    const source = `self.MonacoEnvironment = { baseUrl: "${MONACO}/" };
importScripts("${MONACO}/vs/base/worker/workerMain.js");`;
    return URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
  },
};

const status = document.getElementById("status");
const chartsElement = document.getElementById("charts");

function show(message, error = false) {
  status.textContent = message;
  status.classList.toggle("error", error);
}

async function bytes(url) {
  // The site's files change as the example builds, so each fetch checks the server's copy.
  const response = await fetch(url, { cache: "no-cache" });
  if (!response.ok) throw new Error(`cannot load ${url}: status ${response.status}`);
  return new Uint8Array(await response.arrayBuffer());
}

const loadMonaco = new Promise((resolve) => {
  window.require.config({ paths: { vs: `${MONACO}/vs` } });
  window.require(["vs/editor/editor.main"], () => resolve(window.monaco));
});

const manifest = await (await fetch("manifest.json")).json();
const library = await import("./lib/avenger_sql_web.js");
await library.default({ module_or_path: fetch("./lib/avenger_sql_web_bg.wasm", { cache: "no-cache" }) });
for (const font of manifest.fonts) library.addFonts(await bytes(font));

// The project's files: text for the scripts, the stylesheets, and the manifest, bytes for data.
const TEXT = /\.(sql|sqlm|css|toml|qmd)$/;
const files = {};
for (const path of manifest.files) {
  files[path] = TEXT.test(path) ? new TextDecoder().decode(await bytes(`project/${path}`)) : await bytes(`project/${path}`);
}
show("Loading the project…");
const project = await library.SqlProject.open("vega", files);
const monaco = await loadMonaco;

// The files that the editor edits: the scripts and modules, which avenger-sql-web analyzes, and
// the stylesheets, which style the charts.
const SCRIPT = /\.av\.sqlm?$/;
const editable = manifest.files.filter((path) => SCRIPT.test(path) || path.endsWith(".css"));
const charts = new Set(project.chartScripts());

monaco.languages.register({ id: "avenger-sql", extensions: [".av.sql", ".av.sqlm"] });
monaco.languages.setLanguageConfiguration("avenger-sql", {
  comments: { lineComment: "--", blockComment: ["/*", "*/"] },
  brackets: [
    ["(", ")"],
    ["[", "]"],
  ],
  autoClosingPairs: [
    { open: "(", close: ")" },
    { open: "[", close: "]" },
    { open: "'", close: "'", notIn: ["string"] },
  ],
});

const uri = (path) => monaco.Uri.parse(`file:///${path}`);
const pathOf = (model) => model.uri.path.slice(1);
const models = new Map();
for (const path of editable) {
  const language = SCRIPT.test(path) ? "avenger-sql" : "css";
  models.set(path, monaco.editor.createModel(files[path], language, uri(path)));
}
const modelOf = (path) => models.get(path);

/** The Monaco range of `range`, a range of UTF-16 offsets of `model`. */
function rangeOf(model, range) {
  const start = model.getPositionAt(range.start);
  const end = model.getPositionAt(range.end);
  return new monaco.Range(start.lineNumber, start.column, end.lineNumber, end.column);
}

// The changes reach avenger-sql-web in order, and each request waits for the changes before it,
// so it reads the text that the editor shows.
let pending = Promise.resolve();
const settled = () => pending;
const tokensChanged = new monaco.Emitter();

function changed(model) {
  const path = pathOf(model);
  if (SCRIPT.test(path)) {
    const text = model.getValue();
    pending = pending
      .then(() => project.setText(path, text))
      .catch((error) => show(String(error), true));
  }
  scheduleRefresh();
}

let refreshTimer;
function scheduleRefresh() {
  clearTimeout(refreshTimer);
  refreshTimer = setTimeout(refresh, 250);
}

/** The fixes of each marker, by its model and its range, which the quick fixes offer. */
const fixes = new Map();
const fixKey = (model, range) => `${pathOf(model)}:${range.startLineNumber}:${range.startColumn}:${range.endLineNumber}:${range.endColumn}`;

function publishDiagnostics() {
  fixes.clear();
  for (const [path, model] of models) {
    if (!SCRIPT.test(path)) continue;
    const markers = project.diagnostics(path).map((found) => {
      let range = rangeOf(model, found.range);
      if (range.isEmpty()) range = new monaco.Range(range.startLineNumber, range.startColumn, range.endLineNumber, range.endColumn + 1);
      if (found.fixes.length) fixes.set(fixKey(model, range), found.fixes);
      return {
        severity: found.severity === "error" ? monaco.MarkerSeverity.Error : monaco.MarkerSeverity.Warning,
        message: found.help ? `${found.message}\n${found.help}` : found.message,
        source: found.source,
        startLineNumber: range.startLineNumber,
        startColumn: range.startColumn,
        endLineNumber: range.endLineNumber,
        endColumn: range.endColumn,
      };
    });
    monaco.editor.setModelMarkers(model, "avenger-sql", markers);
  }
}

// The chart script that the preview shows: the open file when it is a chart script, or the last
// one that was open.
let previewed = [...charts][0];
let views = [];

function styles() {
  const css = {};
  for (const [path, model] of models) if (path.endsWith(".css")) css[path] = model.getValue();
  return "styles/main.css" in css ? { entry: "styles/main.css", files: css } : null;
}

async function drawCharts() {
  if (!previewed) return;
  let found;
  try {
    found = await project.charts(previewed);
  } catch (error) {
    show(`${previewed}\n${error.message ?? error}`, true);
    return;
  }
  show(`${previewed}: ${found.length} chart${found.length === 1 ? "" : "s"}`);
  const options = { styles: styles(), colorScheme: "light" };
  if (views.length === found.length) {
    found.forEach((chart, index) => views[index].setChart(chart));
    views.forEach((view) => view.setStyles(options.styles));
    return;
  }
  views.forEach((view) => view.free());
  chartsElement.replaceChildren();
  views = [];
  for (const chart of found) {
    const canvas = document.createElement("canvas");
    canvas.style.width = "560px";
    canvas.style.height = "360px";
    chartsElement.append(canvas);
    views.push(await library.ChartView.create(canvas, chart, options));
  }
}

async function refresh() {
  await settled();
  publishDiagnostics();
  tokensChanged.fire();
  await drawCharts();
}

const editor = monaco.editor.create(document.getElementById("editor"), {
  model: modelOf(previewed),
  theme: "avenger",
  automaticLayout: true,
  minimap: { enabled: false },
  fontSize: 13,
  "semanticHighlighting.enabled": true,
});
for (const model of models.values()) model.onDidChangeContent(() => changed(model));

// The file list, by role.
const nav = document.getElementById("files");
const buttons = new Map();
function open(path) {
  editor.setModel(modelOf(path));
  for (const [other, button] of buttons) button.classList.toggle("active", other === path);
  if (charts.has(path) && path !== previewed) {
    previewed = path;
    views.forEach((view) => view.free());
    views = [];
    chartsElement.replaceChildren();
    drawCharts();
  }
}
const roles = [
  ["Charts", (path) => charts.has(path)],
  ["Data", (path) => path.startsWith("data/") && SCRIPT.test(path)],
  ["Modules", (path) => path.endsWith(".av.sqlm")],
  ["Styles", (path) => path.endsWith(".css")],
];
for (const [role, test] of roles) {
  const heading = document.createElement("div");
  heading.className = "role";
  heading.textContent = role;
  nav.append(heading);
  for (const path of editable.filter(test)) {
    const button = document.createElement("button");
    button.textContent = path;
    button.onclick = () => open(path);
    buttons.set(path, button);
    nav.append(button);
  }
}

// A definition, a reference, or a peek in another file opens that file.
monaco.editor.registerEditorOpener({
  openCodeEditor(_source, resource, selection) {
    const path = resource.path.slice(1);
    if (!models.has(path)) return false;
    open(path);
    if (selection) {
      const range = monaco.Range.isIRange(selection) ? selection : monaco.Range.fromPositions(selection);
      editor.setSelection(range);
      editor.revealRangeInCenter(range);
    }
    return true;
  },
});

// The editor's requests, each by avenger-sql-web.
const COMPLETION = monaco.languages.CompletionItemKind;
function completionKind(kind) {
  if (kind.startsWith("Function")) return COMPLETION.Function;
  return (
    {
      Keyword: COMPLETION.Keyword,
      Column: COMPLETION.Field,
      RangeVariable: COMPLETION.Variable,
      Variable: COMPLETION.Variable,
      Field: COMPLETION.Property,
      Table: COMPLETION.Class,
      Query: COMPLETION.Class,
      Constant: COMPLETION.Constant,
      Parameter: COMPLETION.Property,
      EnumValue: COMPLETION.EnumMember,
      Type: COMPLETION.TypeParameter,
      Window: COMPLETION.Variable,
      Option: COMPLETION.Property,
      File: COMPLETION.File,
      Folder: COMPLETION.Folder,
    }[kind] ?? COMPLETION.Text
  );
}

monaco.languages.registerCompletionItemProvider("avenger-sql", {
  triggerCharacters: [".", ">", "(", ",", "'", "/"],
  async provideCompletionItems(model, position) {
    await settled();
    const found = project.completions(pathOf(model), model.getOffsetAt(position));
    if (!found) return { suggestions: [] };
    const range = rangeOf(model, found.range);
    return {
      suggestions: found.items.map((item) => ({
        label: item.label,
        kind: completionKind(item.kind),
        detail: item.detail ?? undefined,
        documentation: item.documentation ? { value: item.documentation } : undefined,
        insertText: item.insert,
        filterText: item.filter ?? undefined,
        sortText: item.sort,
        range,
      })),
    };
  },
});

monaco.languages.registerHoverProvider("avenger-sql", {
  async provideHover(model, position) {
    await settled();
    const found = project.hover(pathOf(model), model.getOffsetAt(position));
    return found ? { range: rangeOf(model, found.range), contents: [{ value: found.markdown }] } : null;
  },
});

monaco.languages.registerSignatureHelpProvider("avenger-sql", {
  signatureHelpTriggerCharacters: ["(", ","],
  signatureHelpRetriggerCharacters: [","],
  async provideSignatureHelp(model, position) {
    await settled();
    const found = project.signatureHelp(pathOf(model), model.getOffsetAt(position));
    if (!found || !found.signatures.length) return null;
    const active = found.signatures[found.active_signature];
    return {
      value: {
        signatures: found.signatures.map((form) => ({
          label: form.label,
          documentation: form.documentation ? { value: form.documentation } : undefined,
          parameters: form.parameters.map((parameter) => ({ label: [parameter.start, parameter.end] })),
        })),
        activeSignature: found.active_signature,
        activeParameter: active?.active_parameter ?? 0,
      },
      dispose() {},
    };
  },
});

monaco.languages.registerDefinitionProvider("avenger-sql", {
  async provideDefinition(model, position) {
    await settled();
    const found = project.definition(pathOf(model), model.getOffsetAt(position));
    const target = found && modelOf(found.path);
    return target ? { uri: target.uri, range: rangeOf(target, found.target) } : null;
  },
});

monaco.languages.registerReferenceProvider("avenger-sql", {
  async provideReferences(model, position) {
    await settled();
    return project
      .references(pathOf(model), model.getOffsetAt(position))
      .filter((found) => models.has(found.path))
      .map((found) => ({ uri: uri(found.path), range: rangeOf(modelOf(found.path), found.range) }));
  },
});

monaco.languages.registerRenameProvider("avenger-sql", {
  async resolveRenameLocation(model, position) {
    await settled();
    try {
      const range = rangeOf(model, project.prepareRename(pathOf(model), model.getOffsetAt(position)));
      return { range, text: model.getValueInRange(range) };
    } catch (error) {
      return { range: new monaco.Range(position.lineNumber, position.column, position.lineNumber, position.column), text: "", rejectReason: error.message ?? String(error) };
    }
  },
  async provideRenameEdits(model, position, name) {
    await settled();
    try {
      const renamed = project.rename(pathOf(model), model.getOffsetAt(position), name);
      return {
        edits: renamed.flatMap((file) =>
          file.edits.map((edit) => ({
            resource: uri(file.path),
            versionId: undefined,
            textEdit: { range: rangeOf(modelOf(file.path), edit.range), text: edit.text },
          })),
        ),
      };
    } catch (error) {
      return { edits: [], rejectReason: error.message ?? String(error) };
    }
  },
});

monaco.languages.registerDocumentFormattingEditProvider("avenger-sql", {
  async provideDocumentFormattingEdits(model) {
    await settled();
    return project.formatting(pathOf(model)).map((edit) => ({ range: rangeOf(model, edit.range), text: edit.text }));
  },
});

monaco.languages.registerFoldingRangeProvider("avenger-sql", {
  async provideFoldingRanges(model) {
    await settled();
    return project.foldingRanges(pathOf(model)).map((folding) => ({
      start: model.getPositionAt(folding.range.start).lineNumber,
      end: model.getPositionAt(folding.range.end).lineNumber,
      kind: folding.comment ? monaco.languages.FoldingRangeKind.Comment : undefined,
    }));
  },
});

const SYMBOL = monaco.languages.SymbolKind;
const symbolKinds = {
  Variable: SYMBOL.Variable,
  Table: SYMBOL.Class,
  View: SYMBOL.Interface,
  Schema: SYMBOL.Namespace,
  Function: SYMBOL.Function,
  Module: SYMBOL.Module,
  Query: SYMBOL.Object,
  Operator: SYMBOL.Operator,
};
function symbolOf(model, symbol) {
  return {
    name: symbol.name,
    detail: symbol.detail ?? "",
    kind: symbolKinds[symbol.kind] ?? SYMBOL.Object,
    tags: [],
    range: rangeOf(model, symbol.range),
    selectionRange: rangeOf(model, symbol.name_range),
    children: symbol.children.map((child) => symbolOf(model, child)),
  };
}
monaco.languages.registerDocumentSymbolProvider("avenger-sql", {
  async provideDocumentSymbols(model) {
    await settled();
    return project.symbols(pathOf(model)).map((symbol) => symbolOf(model, symbol));
  },
});

monaco.languages.registerCodeActionProvider("avenger-sql", {
  provideCodeActions(model, _range, context) {
    const actions = [];
    for (const marker of context.markers) {
      const range = new monaco.Range(marker.startLineNumber, marker.startColumn, marker.endLineNumber, marker.endColumn);
      for (const fix of fixes.get(fixKey(model, range)) ?? []) {
        actions.push({
          title: fix.label,
          kind: "quickfix",
          diagnostics: [marker],
          isPreferred: true,
          edit: {
            edits: fix.edits.map((edit) => ({
              resource: model.uri,
              versionId: model.getVersionId(),
              textEdit: { range: rangeOf(model, edit.range), text: edit.text },
            })),
          },
        });
      }
    }
    return { actions, dispose() {} };
  },
});

// Semantic highlighting: the classes of avenger-sql-language's highlights, by Monaco's token types.
const TOKEN_TYPES = ["keyword", "comment", "string", "number", "operator", "variable", "parameter", "property", "type", "function", "namespace", "enumMember", "macro"];
function tokenType(kind) {
  if (kind.startsWith("Typst")) return "string";
  if (kind.startsWith("Name(Function")) return "function";
  const type = {
    Keyword: "keyword",
    Comment: "comment",
    String: "string",
    EnumValue: "enumMember",
    Format: "macro",
    Uri: "string",
    Number: "number",
    QueryParameter: "parameter",
    Operator: "operator",
    "Name(Column)": "property",
    "Name(Table)": "type",
    "Name(Schema)": "namespace",
    "Name(Type)": "type",
    "Name(Alias)": "variable",
    "Name(Field)": "property",
    "Name(Window)": "variable",
    "Name(PatternVariable)": "variable",
    "Name(Query)": "type",
    "Name(Parameter)": "parameter",
    "Name(Variable)": "variable",
    "Name(RangeVariable)": "variable",
    "Name(Module)": "namespace",
    "Name(DatePart)": "enumMember",
    "Name(Option)": "parameter",
  }[kind];
  return type === undefined ? -1 : TOKEN_TYPES.indexOf(type);
}

monaco.languages.registerDocumentSemanticTokensProvider("avenger-sql", {
  onDidChange: tokensChanged.event,
  getLegend: () => ({ tokenTypes: TOKEN_TYPES, tokenModifiers: ["declaration"] }),
  async provideDocumentSemanticTokens(model) {
    await settled();
    const data = [];
    let lastLine = 0;
    let lastStart = 0;
    for (const highlight of project.highlights(pathOf(model))) {
      const type = tokenType(highlight.kind);
      if (type < 0) continue;
      // A token spans one line, so a range over lines, such as a block comment, splits.
      let offset = highlight.range.start;
      while (offset < highlight.range.end) {
        const position = model.getPositionAt(offset);
        const lineEnd = model.getOffsetAt({ lineNumber: position.lineNumber, column: model.getLineMaxColumn(position.lineNumber) });
        const end = Math.min(highlight.range.end, lineEnd);
        const line = position.lineNumber - 1;
        const start = position.column - 1;
        if (end > offset) {
          data.push(line - lastLine, line === lastLine ? start - lastStart : start, end - offset, type, highlight.definition ? 1 : 0);
          lastLine = line;
          lastStart = start;
        }
        offset = lineEnd + 1;
      }
    }
    return { data: new Uint32Array(data) };
  },
  releaseDocumentSemanticTokens() {},
});

monaco.editor.defineTheme("avenger", {
  base: "vs",
  inherit: true,
  rules: [
    { token: "keyword", foreground: "8250df", fontStyle: "bold" },
    { token: "comment", foreground: "6e7781", fontStyle: "italic" },
    { token: "string", foreground: "0a3069" },
    { token: "number", foreground: "0550ae" },
    { token: "operator", foreground: "cf222e" },
    { token: "function", foreground: "6639ba" },
    { token: "type", foreground: "953800" },
    { token: "namespace", foreground: "953800" },
    { token: "property", foreground: "0550ae" },
    { token: "parameter", foreground: "116329" },
    { token: "variable", foreground: "1f2328" },
    { token: "enumMember", foreground: "0550ae" },
    { token: "macro", foreground: "0550ae" },
  ],
  colors: {},
});
monaco.editor.setTheme("avenger");

open(previewed);
await refresh();
