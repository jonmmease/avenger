import { spawn } from 'node:child_process';

/**
 * The avenger engine of Quarto 2: the `{avenger-sql}` cells of a document run through
 * `avenger quarto2-engine`, which draws their charts, and q2's core writes the cells and their
 * figures from a notebook record of the outputs, by q2's unit ids.
 *
 * `avenger` is the program on PATH, or the path in the AVENGER environment variable. The request
 * and the response are JSON, in the protocol that `PROTOCOL` names, which avenger checks.
 */

             
                      
                     
                       
                 
                
                           
                          
                  
         
               
                      
                     
            
                       

/** The protocol of the requests to `avenger quarto2-engine` and of its responses. */
const PROTOCOL = 1;

/** A unit of a document that q2 gives its engines: a cell, by its id and its fence's line. */
                       
             
                
               
                   
               
                 
 

/** An nbformat 4 output. */
                                             

/** A cell of avenger's response: its number, the line of its fence, and its outputs. */
                               
                 
               
                    
 

/** A color scheme of a document's themes. */
                                           

/** The request of `avenger quarto2-engine`. */
                          
                   
                   
               
                                                  
                              
                 
 

/** The output formats whose images are SVG. Others, such as docx and pptx, take PNG. */
const SVG_FORMATS = new Set([
  "html",
  "html4",
  "html5",
  "revealjs",
  "dashboard",
  "typst",
  "gfm",
  "commonmark",
  "commonmark_x",
  "markdown",
]);

/** The image of a cell without its own `fig-format`, by the output format `to`. */
function imageFor(to        )                   {
  const base = to.split(/[+-]/)[0];
  return SVG_FORMATS.has(base) ? { format: "svg", scale: 1 } : { format: "png", scale: 2 };
}

/** The output formats whose pages switch between a light and a dark theme with q2's toggle. */
const TOGGLE_FORMATS = new Set(["html", "html4", "html5", "dashboard"]);

/**
 * The color schemes of the charts of a document whose `theme` is `theme`, in the output format
 * `to`: light and dark, in that order, for a theme with a dark variant in a format with q2's
 * toggle, since q2 adds its default light variant to a theme of a dark one alone. A theme's name,
 * a theme of a light variant alone, no theme, and the formats without the toggle give light.
 */
function colorSchemesFor(theme         , to        )                {
  const dark = TOGGLE_FORMATS.has(to.split(/[+-]/)[0]) &&
    theme !== null && typeof theme === "object" && !Array.isArray(theme) && "dark" in theme;
  return dark ? ["light", "dark"] : ["light"];
}

/** The options that only change how q2's core writes a cell, never what avenger computes. */
const PRESENTATION_OPTIONS = [
  "echo",
  "output",
  "warning",
  "include",
  "code-fold",
  "code-summary",
  "code-overflow",
  "code-line-numbers",
  "code-copy",
  "fig-cap",
  "fig-subcap",
  "fig-scap",
  "fig-alt",
  "fig-align",
  "fig-link",
  "fig-cap-location",
  "cap-location",
  "label",
  "layout",
  "layout-ncol",
  "layout-nrow",
  "layout-align",
  "layout-valign",
  "panel",
  "column",
  "fig-env",
  "fig-pos",
  "out-width",
  "out-height",
  "classes",
  "fig-column",
];

/**
 * The notebook record of the owned cells `units` with the outputs of `cells`, avenger's response,
 * for q2's core to write. The response's cells must be the units, by count and by line, or q2 and
 * avenger disagree on what a cell is.
 */
function record(
  units        ,
  cells                ,
)                                             {
  const lines = (list                    ) => list.map((item) => item.line).join(", ");
  const disagree = units.length !== cells.length ||
    units.some((unit, index) => unit.line !== cells[index].line);
  if (disagree) {
    throw new Error(
      `q2 and avenger find different cells: q2's at lines ${lines(units)}, avenger's at lines ` +
        `${lines(cells)}`,
    );
  }
  return {
    notebook: {
      nbformat: 4,
      nbformat_minor: 5,
      metadata: {
        kernelspec: { name: "avenger", language: "avenger-sql", display_name: "Avenger SQL" },
      },
      cells: units.map((unit, index) => ({
        cell_type: "code",
        execution_count: index + 1,
        metadata: { quarto_cell_id: unit.id },
        outputs: cells[index].outputs,
        source: [],
      })),
    },
    recordInfo: {
      version: 1,
      granularity: "unit",
      outputKind: "raw",
      alignment: "ids",
      presentedBy: "core",
      dependencyData: "jupyter-outputs",
      hashDefinition: {
        id: "avenger",
        version: 1,
        // A cell reads the tables and the variables of the cells before it.
        granularity: "document",
        presentationCellOptions: PRESENTATION_OPTIONS,
        presentationExecuteKeys: PRESENTATION_OPTIONS,
        documentKeys: ["fig-width", "fig-height", "fig-format", "fig-dpi"],
      },
    },
  };
}

/**
 * The cells of avenger's response to `request`, from `avenger quarto2-engine` with `env` added to
 * its environment, or avenger's error.
 */
function runAvenger(
  request         ,
  env                         = {},
)                          {
  const program = process.env.AVENGER || "avenger";
  return new Promise((resolve, reject) => {
    const child = spawn(program, ["quarto2-engine"], {
      env: { ...process.env, ...env },
      stdio: ["pipe", "pipe", "pipe"],
    });
    // UTF-8 decodes a character whose bytes two chunks split, as `chunk + chunk` would not.
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    let stdout = "";
    let stderr = "";
    let failed = false;
    child.stdout.on("data", (chunk        ) => (stdout += chunk));
    child.stderr.on("data", (chunk        ) => (stderr += chunk));
    child.on("error", (error                       ) => {
      failed = true;
      reject(
        error.code === "ENOENT"
          ? new Error("the avenger engine needs `avenger` on PATH, or its path in AVENGER")
          : error,
      );
    });
    // A process that didn't start closes too, after its error.
    child.on("close", () => {
      if (failed) {
        return;
      }
      let response                                            ;
      try {
        response = JSON.parse(stdout);
      } catch {
        reject(new Error(`avenger answered no response: ${stderr.trim() || stdout.trim()}`));
        return;
      }
      if (response.error !== undefined || response.cells === undefined) {
        reject(new Error(response.error ?? "avenger answered no cells"));
        return;
      }
      resolve(response.cells);
    });
    child.stdin.end(JSON.stringify(request));
  });
}

/** The engine's `execute`: the record of the document's avenger cells, which q2's core writes. */
async function execute(options                )                         {
  // q2 sends `units` and `presentedBy`, which the published @quarto/types doesn't declare yet.
  const offered = options                                                             ;
  const units = (offered.units ?? []).filter((unit) => unit.kind === "cell" && unit.owned);
  const markdown = options.target.markdown.value;
  if (units.length === 0) {
    return { markdown, supporting: [], filters: [] };
  }
  if (offered.presentedBy !== "core") {
    throw new Error("the avenger engine needs a q2 that writes the cells of an engine's record");
  }
  const to = options.format.pandoc.to ?? "html";
  const request          = {
    protocol: PROTOCOL,
    document: options.target.input,
    text: markdown,
    image: imageFor(to),
    colorSchemes: colorSchemesFor(options.format.metadata?.theme, to),
    error: options.format.execute.error === true,
  };
  const env = (options                                                            ).executeEnv;
  const cells = await runAvenger(request, env ?? {});
  return { markdown: "", supporting: [], filters: [], ...record(units, cells) }                 ;
}

let quarto                       ;

const engine                           = {
  name: "avenger",
  defaultExt: ".qmd",
  defaultYaml: () => [],
  defaultContent: () => [],
  validExtensions: () => [],
  canFreeze: false,
  generatesFigures: true,

  init(api           )       {
    quarto = api;
  },

  claimsLanguage(language        )          {
    return language === "avenger-sql";
  },

  claimsFile()          {
    return false;
  },

  launch(_context                      )                          {
    const markdownForFile = (file        )                        =>
      Promise.resolve(quarto .mappedString.fromFile(file));
    return {
      name: "avenger",
      canFreeze: false,
      markdownForFile,
      async target(file        , _quiet          , markdown               ) {
        const ms = markdown ?? (await markdownForFile(file));
        return { source: file, input: file, markdown: ms, metadata: {} }                   ;
      },
      async partitionedMarkdown(file        , _format         )                               {
        const ms = await markdownForFile(file);
        return {
          markdown: ms.value,
          yaml: undefined,
          headingText: undefined,
          headingAttr: undefined,
          containsRefs: false,
          srcMarkdownNoYaml: ms.value,
        };
      },
      execute,
      async dependencies(_options                     )                              {
        return { includes: {} };
      },
      async postprocess(_options                    )                {},
    };
  },
};

export { PROTOCOL, colorSchemesFor, engine as default, execute, imageFor, record, runAvenger };
