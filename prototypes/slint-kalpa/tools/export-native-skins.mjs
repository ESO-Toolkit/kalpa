import fs from "node:fs";
import path from "node:path";
import ts from "typescript";

const repoRoot = path.resolve(import.meta.dirname, "../../..");
const sourcePath = path.join(repoRoot, "src/lib/theme-skins.ts");
const outDir = path.join(repoRoot, "prototypes/slint-kalpa/assets/skins");

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
  return unwrap(found);
}

function unwrap(node) {
  if (ts.isParenthesizedExpression(node)) return unwrap(node.expression);
  if (ts.isAsExpression(node) || ts.isSatisfiesExpression?.(node)) return unwrap(node.expression);
  return node;
}

function propName(name) {
  if (ts.isIdentifier(name) || ts.isStringLiteral(name)) return name.text;
  throw new Error(`Unsupported property name: ${name.getText(ast)}`);
}

function objectProp(objectNode, key) {
  const object = unwrap(objectNode);
  if (!ts.isObjectLiteralExpression(object)) throw new Error(`Expected object for ${key}`);
  for (const prop of object.properties) {
    if (ts.isPropertyAssignment(prop) && propName(prop.name) === key) return unwrap(prop.initializer);
  }
  return undefined;
}

function literal(node) {
  const value = unwrap(node);
  if (ts.isIdentifier(value)) return literal(declaration(value.text));
  if (ts.isStringLiteral(value) || ts.isNoSubstitutionTemplateLiteral(value)) return value.text;
  if (ts.isNumericLiteral(value)) return Number(value.text);
  if (ts.isArrayLiteralExpression(value)) return value.elements.map(literal);
  if (
    ts.isCallExpression(value) &&
    ts.isPropertyAccessExpression(value.expression) &&
    value.expression.name.text === "join" &&
    ts.isArrayLiteralExpression(value.expression.expression)
  ) {
    return value.expression.expression.elements.map(literal).join(literal(value.arguments[0]));
  }
  throw new Error(`Unsupported literal: ${value.getText(ast).slice(0, 120)}`);
}

function svgConstNameFromCall(node) {
  const value = unwrap(node);
  if (
    ts.isCallExpression(value) &&
    ts.isIdentifier(value.expression) &&
    value.expression.text === "svgUrl" &&
    ts.isIdentifier(value.arguments[0])
  ) {
    return value.arguments[0].text;
  }
  throw new Error(`Expected svgUrl(CONST), got ${value.getText(ast).slice(0, 120)}`);
}

function textureConstName(node) {
  const value = unwrap(node);
  if (
    ts.isCallExpression(value) &&
    ts.isPropertyAccessExpression(value.expression) &&
    value.expression.name.text === "join" &&
    ts.isArrayLiteralExpression(value.expression.expression)
  ) {
    const svgCalls = value.expression.expression.elements
      .map(unwrap)
      .filter(
        (entry) =>
          ts.isCallExpression(entry) &&
          ts.isIdentifier(entry.expression) &&
          entry.expression.text === "svgUrl"
      );
    if (svgCalls.length > 0) return svgConstNameFromCall(svgCalls[svgCalls.length - 1]);
  }
  return svgConstNameFromCall(value);
}

function sizeFromCss(value, fallbackSvg) {
  if (typeof value === "string") {
    const match = value.match(/(\d+(?:\.\d+)?)px\s+(\d+(?:\.\d+)?)px/);
    if (match) return [Number(match[1]), Number(match[2])];
  }
  return svgSize(fallbackSvg);
}

function svgSize(svg) {
  const width = svg.match(/\bwidth="([0-9.]+)"/)?.[1];
  const height = svg.match(/\bheight="([0-9.]+)"/)?.[1];
  if (!width || !height) throw new Error(`Could not read SVG size from ${svg.slice(0, 80)}`);
  return [Number(width), Number(height)];
}

function generatedSkinSvg({ textureSvg, textureSize, patternSvg, patternSize, patternOpacity }) {
  const [textureWidth, textureHeight] = sizeFromCss(textureSize, textureSvg);
  const [patternWidth, patternHeight] = sizeFromCss(patternSize, patternSvg);
  const motifOpacity = patternOpacity ?? 0.12;

  return `<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="800" viewBox="0 0 1200 800">
  <defs>
    <pattern id="native-texture" width="${textureWidth}" height="${textureHeight}" patternUnits="userSpaceOnUse">
      ${textureSvg}
    </pattern>
    <pattern id="native-pattern" width="${patternWidth}" height="${patternHeight}" patternUnits="userSpaceOnUse">
      ${patternSvg}
    </pattern>
  </defs>
  <rect width="1200" height="800" fill="url(#native-texture)" opacity="0.56"/>
  <rect width="1200" height="800" fill="url(#native-pattern)" opacity="${motifOpacity}"/>
</svg>
`;
}

const skins = declaration("SKINS");
if (!ts.isObjectLiteralExpression(skins)) throw new Error("SKINS must be an object literal");

fs.mkdirSync(outDir, { recursive: true });

let written = 0;
for (const prop of skins.properties) {
  if (!ts.isPropertyAssignment(prop)) continue;
  const id = propName(prop.name);
  const skin = unwrap(prop.initializer);
  const texture = objectProp(skin, "texture");
  const textureSize = objectProp(skin, "textureSize");
  const pattern = objectProp(skin, "pattern");
  const patternSize = objectProp(skin, "patternSize");
  const patternOpacity = objectProp(skin, "patternOpacity");

  if (!texture || !pattern) continue;

  const textureSvg = literal(declaration(textureConstName(texture)));
  const patternSvg = literal(declaration(svgConstNameFromCall(pattern)));
  const svg = generatedSkinSvg({
    textureSvg,
    textureSize: textureSize ? literal(textureSize) : undefined,
    patternSvg,
    patternSize: patternSize ? literal(patternSize) : undefined,
    patternOpacity: patternOpacity ? literal(patternOpacity) : undefined,
  });

  fs.writeFileSync(path.join(outDir, `${id}.svg`), svg);
  written += 1;
}

console.log(`wrote ${path.relative(repoRoot, outDir)} (${written} skins)`);
