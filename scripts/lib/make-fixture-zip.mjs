/**
 * A dependency-free ZIP writer, just big enough to build the addon fixtures the
 * sandboxed e2e suite installs.
 *
 * Everything is stored (compression method 0), which the Rust `zip` crate reads
 * the same as a deflated entry — the point of the fixture is to exercise Kalpa's
 * extraction, folder-wrap and rollback paths, not its inflater. Writing the
 * archive here rather than checking one in keeps the fixture readable in the
 * diff and lets a spec vary it (flat archive, nested archive, extra files)
 * without adding binaries to the repo.
 */

import { crc32 } from "node:zlib";

const SIGNATURE_LOCAL = 0x04034b50;
const SIGNATURE_CENTRAL = 0x02014b50;
const SIGNATURE_END = 0x06054b50;
const VERSION_NEEDED = 20;
const METHOD_STORED = 0;

/**
 * Build a ZIP from `{ [entryPath]: contents }`.
 *
 * Entry paths use forward slashes — the ZIP spec requires it, and a backslash
 * would land as a literal character in the file name on extraction.
 */
export function makeZip(entries) {
  const localChunks = [];
  const centralChunks = [];
  let offset = 0;

  for (const [name, contents] of Object.entries(entries)) {
    if (name.includes("\\")) {
      throw new Error(`ZIP entry names must use forward slashes: ${name}`);
    }
    const nameBytes = Buffer.from(name, "utf8");
    const data = Buffer.isBuffer(contents) ? contents : Buffer.from(contents, "utf8");
    const checksum = crc32(data);

    const local = Buffer.alloc(30);
    local.writeUInt32LE(SIGNATURE_LOCAL, 0);
    local.writeUInt16LE(VERSION_NEEDED, 4);
    local.writeUInt16LE(0, 6); // flags
    local.writeUInt16LE(METHOD_STORED, 8);
    local.writeUInt16LE(0, 10); // mod time
    local.writeUInt16LE(0, 12); // mod date
    local.writeUInt32LE(checksum, 14);
    local.writeUInt32LE(data.length, 18); // compressed size
    local.writeUInt32LE(data.length, 22); // uncompressed size
    local.writeUInt16LE(nameBytes.length, 26);
    local.writeUInt16LE(0, 28); // extra length
    localChunks.push(local, nameBytes, data);

    const central = Buffer.alloc(46);
    central.writeUInt32LE(SIGNATURE_CENTRAL, 0);
    central.writeUInt16LE(VERSION_NEEDED, 4); // version made by
    central.writeUInt16LE(VERSION_NEEDED, 6);
    central.writeUInt16LE(0, 8); // flags
    central.writeUInt16LE(METHOD_STORED, 10);
    central.writeUInt16LE(0, 12);
    central.writeUInt16LE(0, 14);
    central.writeUInt32LE(checksum, 16);
    central.writeUInt32LE(data.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(nameBytes.length, 28);
    central.writeUInt16LE(0, 30); // extra
    central.writeUInt16LE(0, 32); // comment
    central.writeUInt16LE(0, 34); // disk number
    central.writeUInt16LE(0, 36); // internal attrs
    central.writeUInt32LE(0, 38); // external attrs
    central.writeUInt32LE(offset, 42);
    centralChunks.push(central, nameBytes);

    offset += local.length + nameBytes.length + data.length;
  }

  const centralDirectory = Buffer.concat(centralChunks);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(SIGNATURE_END, 0);
  end.writeUInt16LE(0, 4); // disk number
  end.writeUInt16LE(0, 6); // central directory start disk
  end.writeUInt16LE(Object.keys(entries).length, 8);
  end.writeUInt16LE(Object.keys(entries).length, 10);
  end.writeUInt32LE(centralDirectory.length, 12);
  end.writeUInt32LE(offset, 16);
  end.writeUInt16LE(0, 20); // comment length

  return Buffer.concat([...localChunks, centralDirectory, end]);
}

/**
 * A minimal but realistic ESO addon: the `.txt` manifest ESO reads plus one Lua
 * file, nested under the addon's own folder the way an ESOUI download is.
 */
export function makeAddonZip(folderName, { title = folderName, version = "1.0" } = {}) {
  const manifest = [
    "## Title: " + title,
    "## Author: Kalpa E2E",
    "## Version: " + version,
    "## APIVersion: 101044",
    "## Description: Fixture addon installed by the sandboxed e2e suite.",
    "",
    folderName + ".lua",
    "",
  ].join("\n");

  return makeZip({
    [`${folderName}/${folderName}.txt`]: manifest,
    [`${folderName}/${folderName}.lua`]: `-- ${title} fixture\nlocal _ = ${version}\n`,
  });
}
