import type {
  SvTreeNode,
  WidgetType,
  WidgetConfidence,
  WidgetProps,
  NodeContext,
  EffectiveField,
  SvSchemaOverlay,
} from "../types";
import { classifyContext, humanizeKey } from "./sv-nodes";

/**
 * Check if a table node represents an RGB(A) color (all values numbers 0–1).
 */
function isColorTable(node: SvTreeNode): boolean {
  if (!node.children || node.children.length < 3 || node.children.length > 4) return false;

  const keys = new Set(node.children.map((c) => c.key));
  const hasRGB = keys.has("r") && keys.has("g") && keys.has("b");
  if (!hasRGB) return false;

  // Allow only r, g, b, and optionally a
  const allowedKeys = new Set(["r", "g", "b", "a"]);
  for (const child of node.children) {
    if (!allowedKeys.has(child.key)) return false;
    if (child.valueType !== "number") return false;
    const v = child.value as number;
    if (v < 0 || v > 1) return false;
  }
  return true;
}

/**
 * Infer widget type, confidence, and default props from a tree node.
 */
export function inferWidget(node: SvTreeNode): {
  widget: WidgetType;
  confidence: WidgetConfidence;
  props: WidgetProps;
} {
  // Nil values → readonly
  if (node.valueType === "nil") {
    return { widget: "readonly", confidence: "certain", props: {} };
  }

  // Boolean → toggle (certain)
  if (node.valueType === "boolean") {
    return { widget: "toggle", confidence: "certain", props: {} };
  }

  // Table: check for color pattern first, then group
  if (node.valueType === "table" && node.children) {
    if (isColorTable(node)) {
      return { widget: "color", confidence: "certain", props: {} };
    }
    return { widget: "group", confidence: "certain", props: {} };
  }

  // Number → number input (inferred, never slider by default)
  if (node.valueType === "number") {
    return { widget: "number", confidence: "inferred", props: {} };
  }

  // String → text input (inferred), auto-textarea if long
  if (node.valueType === "string") {
    const multiline = typeof node.value === "string" && node.value.length > 80;
    return { widget: "text", confidence: "inferred", props: { multiline } };
  }

  // Fallback
  return { widget: "raw", confidence: "ambiguous", props: {} };
}

/**
 * Resolve the effective field for a node by merging inference with user overlay.
 *
 * This is the single source of truth for how any node renders.
 */
export function resolveEffectiveField(
  node: SvTreeNode,
  pathSegments: string[],
  context: NodeContext,
  overlay: SvSchemaOverlay,
  addonName: string,
  knownCharacters: Set<string>
): EffectiveField {
  // pathSegments already includes the addon name as the first element
  const nodeId = pathSegments.join("/");
  const inferred = inferWidget(node);

  // Start with inferred values
  let widget = inferred.widget;
  let confidence = inferred.confidence;
  let props: WidgetProps = { ...inferred.props };
  let hidden = false;
  let readOnly = false;
  let label = humanizeKey(node.key);

  // Apply overlay if it exists
  const addonOverlay = overlay[addonName];
  if (addonOverlay) {
    const override = addonOverlay[nodeId];
    if (override) {
      if (override.widget !== undefined) {
        widget = override.widget;
        confidence = "certain"; // user chose it explicitly
      }
      if (override.props) {
        props = { ...props, ...override.props };
      }
      if (override.hidden !== undefined) hidden = override.hidden;
      if (override.readOnly !== undefined) readOnly = override.readOnly;
      if (override.label !== undefined) label = override.label;
    }
  }

  // A number only becomes a slider if effective schema has both min and max
  if (widget === "slider" && (props.min === undefined || props.max === undefined)) {
    widget = "number";
  }

  // Resolve children recursively for group/table nodes
  let children: EffectiveField[] | undefined;
  if (node.valueType === "table" && node.children && widget !== "color") {
    children = node.children.map((child) => {
      const childPath = [...pathSegments, child.key];
      const childContext = classifyContext(child.key, pathSegments.length, knownCharacters);
      return resolveEffectiveField(
        child,
        childPath,
        childContext,
        overlay,
        addonName,
        knownCharacters
      );
    });
  }

  return {
    nodeId,
    key: node.key,
    label,
    widget,
    confidence,
    context,
    props,
    hidden,
    readOnly,
    value: node.value ?? null,
    children,
  };
}
