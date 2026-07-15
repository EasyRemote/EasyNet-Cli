import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import ts from "../node/node_modules/typescript/lib/typescript.js";

const input = path.resolve(process.argv[2] ?? "sdk/node/index.d.ts");
const source = ts.createSourceFile(
  input,
  fs.readFileSync(input, "utf8"),
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TS,
);
const printer = ts.createPrinter({ removeComments: true });
const symbols = new Set();
const members = new Set();
const shapes = {};

function normalized(node) {
  return printer.printNode(ts.EmitHint.Unspecified, node, source).replace(/\s+/g, " ").trim();
}

function isExported(node) {
  return Boolean(node.modifiers?.some((modifier) => modifier.kind === ts.SyntaxKind.ExportKeyword));
}

function declarationName(node) {
  return node.name && ts.isIdentifier(node.name) ? node.name.text : null;
}

function memberName(node) {
  if (ts.isConstructorDeclaration(node) || ts.isConstructSignatureDeclaration(node)) return "new";
  if (ts.isCallSignatureDeclaration(node)) return "call";
  if (ts.isIndexSignatureDeclaration(node)) return "index";
  if (node.name && ts.isComputedPropertyName(node.name)) {
    return normalized(node.name).replace(/^\[/, "").replace(/\]$/, "");
  }
  if (!node.name) return null;
  if (ts.isIdentifier(node.name) || ts.isStringLiteral(node.name) || ts.isNumericLiteral(node.name)) {
    return node.name.text;
  }
  return null;
}

for (const statement of source.statements) {
  if (!isExported(statement)) continue;
  if (ts.isExportDeclaration(statement)) {
    if (!statement.exportClause || !ts.isNamedExports(statement.exportClause)) {
      throw new Error(`wildcard export is not inventory-safe: ${normalized(statement)}`);
    }
    for (const element of statement.exportClause.elements) {
      symbols.add(element.name.text);
      shapes[element.name.text] = normalized(element);
    }
    continue;
  }
  if (ts.isVariableStatement(statement)) {
    for (const declaration of statement.declarationList.declarations) {
      if (!ts.isIdentifier(declaration.name)) {
        throw new Error(`destructured public declaration is not inventory-safe: ${normalized(declaration)}`);
      }
      symbols.add(declaration.name.text);
      shapes[declaration.name.text] = normalized(declaration);
    }
    continue;
  }
  const name = declarationName(statement);
  if (!name) {
    throw new Error(`unnamed public declaration is not inventory-safe: ${normalized(statement)}`);
  }
  symbols.add(name);
  shapes[name] = normalized(statement);
  if (ts.isClassDeclaration(statement) || ts.isInterfaceDeclaration(statement) || ts.isEnumDeclaration(statement)) {
    for (const member of statement.members) {
      const child = memberName(member);
      if (!child) {
        throw new Error(`computed public member is not inventory-safe: ${name}: ${normalized(member)}`);
      }
      let id = `${name}.${child}`;
      if (members.has(id)) {
        id = `${id}#${node.pos}`;
      }
      members.add(id);
      shapes[id] = normalized(member);
    }
  }
}

process.stdout.write(`${JSON.stringify({
  parser: `typescript-${ts.version}`,
  symbols: [...symbols].sort(),
  members: [...members].sort(),
  shapes: Object.fromEntries(Object.entries(shapes).sort(([a], [b]) => a.localeCompare(b))),
})}\n`);
