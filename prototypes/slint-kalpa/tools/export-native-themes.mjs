import fs from "node:fs";
import path from "node:path";
import ts from "typescript";

const repoRoot = path.resolve(import.meta.dirname, "../../..");
const sourcePath = path.join(repoRoot, "src/lib/theme-presets.ts");
const outPath = path.join(repoRoot, "prototypes/slint-kalpa/assets/themes/builtin-themes.json");

const source = fs.readFileSync(sourcePath, "utf8");
const ast = ts.createSourceFile(sourcePath, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);

function declaration(name) {
  let found;
  ast.forEachChild((node) => {
    if (!ts.isVariableStatement(node)) return;
    for (const decl of node.declarationList.declarations) {
      if (ts.isIdentifier(decl.name) && decl.name.text === name) found = decl.initializer;
    }
  });
  if (!found) throw new Error(`Could not find ${name}`);
  return found;
}

function propName(name) {
  if (ts.isIdentifier(name) || ts.isStringLiteral(name)) return name.text;
  throw new Error(`Unsupported property name: ${name.getText(ast)}`);
}

function literal(node) {
  if (ts.isIdentifier(node)) return literal(declaration(node.text));
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) return node.text;
  if (ts.isNumericLiteral(node)) return Number(node.text);
  if (node.kind === ts.SyntaxKind.TrueKeyword) return true;
  if (node.kind === ts.SyntaxKind.FalseKeyword) return false;
  if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "seed") {
    return literal(node.arguments[0]);
  }
  if (ts.isElementAccessExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "SKINS") {
    return { skinId: literal(node.argumentExpression) };
  }
  if (ts.isObjectLiteralExpression(node)) {
    const value = {};
    for (const prop of node.properties) {
      if (!ts.isPropertyAssignment(prop)) continue;
      const key = propName(prop.name);
      value[key] = literal(prop.initializer);
    }
    return value;
  }
  if (ts.isArrayLiteralExpression(node)) return node.elements.map(literal);
  throw new Error(`Unsupported literal: ${node.getText(ast).slice(0, 120)}`);
}

function normalizeTheme(theme) {
  const skinId = theme.skin?.skinId;
  return {
    id: theme.id,
    name: theme.name,
    category: theme.category,
    description: theme.description,
    colors: theme.colors,
    ...(skinId ? { skinId } : {}),
  };
}

const root = normalizeTheme(literal(declaration("ROOT_THEME")));
const generated = literal(declaration("GENERATED_THEMES")).map(normalizeTheme);
const payload = {
  defaultThemeId: literal(declaration("DEFAULT_THEME_ID")),
  rootThemeId: literal(declaration("ROOT_THEME_ID")),
  themes: [root, ...generated],
};

fs.mkdirSync(path.dirname(outPath), { recursive: true });
fs.writeFileSync(outPath, `${JSON.stringify(payload, null, 2)}\n`);
console.log(`wrote ${path.relative(repoRoot, outPath)} (${payload.themes.length} themes)`);
