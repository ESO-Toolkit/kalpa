import type { PackAddonEntry, ValidationError } from "./types";

const VALID_TYPES = ["addon-pack", "build-pack", "roster-pack"];
const VALID_STATUSES = ["draft", "published"];
const ID_PATTERN = /^[a-z0-9-]+$/;
const MAX_ID_LENGTH = 100;
const MAX_NAME_LENGTH = 100;
const MAX_DESCRIPTION_LENGTH = 1000;
const MAX_TAGS = 10;
const MAX_ADDONS = 200;

/**
 * Hard ceiling on any request body, applied before JSON.parse.
 *
 * The per-field caps below bound the fields we know about, but nothing bounded
 * the payload as a whole: an authed caller could attach megabytes of junk that
 * validated fine, got persisted, and (for packs) went into the shared
 * `index:packs` value, which fails to write once it crosses KV's 25MB limit.
 * A fully-populated 200-addon pack is well under 100KB.
 */
export const MAX_BODY_BYTES = 256_000;

export type JsonBodyResult =
  | { ok: true; body: unknown }
  | { ok: false; reason: "too-large" | "invalid-json" };

/** Read a request body as JSON, refusing anything over MAX_BODY_BYTES. */
export async function readJsonBody(request: Request): Promise<JsonBodyResult> {
  const declared = Number(request.headers.get("Content-Length"));
  if (Number.isFinite(declared) && declared > MAX_BODY_BYTES) {
    return { ok: false, reason: "too-large" };
  }
  const text = await request.text();
  if (text.length > MAX_BODY_BYTES) {
    return { ok: false, reason: "too-large" };
  }
  try {
    return { ok: true, body: JSON.parse(text) };
  } catch {
    return { ok: false, reason: "invalid-json" };
  }
}

/**
 * Rebuild addon entries from only the fields we validate.
 *
 * validatePack checks known fields but says nothing about unknown ones, so
 * storing the client's objects by reference persisted (and served back) any
 * extra property they carried. Callers must run validatePack first — this
 * assumes the shape is already known-good and only strips the extras.
 */
export function sanitizeAddons(addons: unknown): PackAddonEntry[] {
  return (addons as Record<string, unknown>[]).map((addon) => {
    const entry: PackAddonEntry = {
      esouiId: addon.esouiId as number,
      name: addon.name as string,
      required: addon.required as boolean,
    };
    if (typeof addon.defaultEnabled === "boolean") entry.defaultEnabled = addon.defaultEnabled;
    if (typeof addon.note === "string") entry.note = addon.note;
    return entry;
  });
}

/** Validate a create/update pack payload from the Rust client. */
export function validatePack(pack: unknown): ValidationError[] {
  const errors: ValidationError[] = [];

  if (!pack || typeof pack !== "object") {
    return [{ field: "pack", message: "Pack must be a JSON object" }];
  }

  const p = pack as Record<string, unknown>;

  // id is optional on create (server generates), required on update
  if (p.id !== undefined) {
    if (typeof p.id !== "string" || p.id.length === 0 || p.id.length > MAX_ID_LENGTH || !ID_PATTERN.test(p.id)) {
      errors.push({
        field: "id",
        message: `id must be 1-${MAX_ID_LENGTH} characters, lowercase letters, numbers, and hyphens`,
      });
    }
  }

  if (typeof p.title !== "string" || p.title.length === 0 || p.title.length > MAX_NAME_LENGTH) {
    errors.push({
      field: "title",
      message: `title is required and must be 1-${MAX_NAME_LENGTH} characters`,
    });
  }

  if (typeof p.description !== "string" || p.description.length > MAX_DESCRIPTION_LENGTH) {
    errors.push({
      field: "description",
      message: `description must be a string under ${MAX_DESCRIPTION_LENGTH} characters`,
    });
  }

  if (typeof p.pack_type !== "string" || !VALID_TYPES.includes(p.pack_type)) {
    errors.push({
      field: "pack_type",
      message: `pack_type must be one of: ${VALID_TYPES.join(", ")}`,
    });
  }

  if (p.status !== undefined && (typeof p.status !== "string" || !VALID_STATUSES.includes(p.status))) {
    errors.push({
      field: "status",
      message: `status must be one of: ${VALID_STATUSES.join(", ")}`,
    });
  }

  if (!Array.isArray(p.tags) || p.tags.length > MAX_TAGS) {
    errors.push({
      field: "tags",
      message: `tags must be an array with at most ${MAX_TAGS} entries`,
    });
  } else {
    for (let i = 0; i < p.tags.length; i++) {
      if (typeof p.tags[i] !== "string" || p.tags[i].length === 0 || p.tags[i].length > 50) {
        errors.push({
          field: `tags[${i}]`,
          message: "each tag must be a non-empty string of at most 50 characters",
        });
        break;
      }
    }
  }

  if (!Array.isArray(p.addons) || p.addons.length === 0 || p.addons.length > MAX_ADDONS) {
    errors.push({
      field: "addons",
      message: `addons must be an array with 1 to ${MAX_ADDONS} entries`,
    });
  } else {
    for (let i = 0; i < p.addons.length; i++) {
      const addon = p.addons[i] as Record<string, unknown>;
      if (typeof addon.esouiId !== "number" || !Number.isInteger(addon.esouiId) || addon.esouiId <= 0) {
        errors.push({
          field: `addons[${i}].esouiId`,
          message: "esouiId must be a positive number",
        });
      }
      if (typeof addon.name !== "string" || addon.name.length === 0 || addon.name.length > 200) {
        errors.push({
          field: `addons[${i}].name`,
          message: "name is required and must be at most 200 characters",
        });
      }
      if (addon.note !== undefined && (typeof addon.note !== "string" || addon.note.length > 500)) {
        errors.push({
          field: `addons[${i}].note`,
          message: "note must be a string of at most 500 characters",
        });
      }
      if (typeof addon.required !== "boolean") {
        errors.push({
          field: `addons[${i}].required`,
          message: "required must be a boolean",
        });
      }
      if (addon.defaultEnabled !== undefined && typeof addon.defaultEnabled !== "boolean") {
        errors.push({
          field: `addons[${i}].defaultEnabled`,
          message: "defaultEnabled must be a boolean",
        });
      }
    }
  }

  return errors;
}
