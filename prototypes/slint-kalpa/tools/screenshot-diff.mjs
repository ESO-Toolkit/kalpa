#!/usr/bin/env node
import { existsSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { deflateSync, inflateSync } from "node:zlib";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "../../..");
const defaultBaseline = path.join(repoRoot, ".screenshots", "main-desktop.png");
const defaultOutputDir = path.join(scriptDir, "diff-output");
const pngSignature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

function usage() {
  console.log(`Usage:
  node prototypes/slint-kalpa/tools/screenshot-diff.mjs <native-screenshot.png> [options]

Compares a captured Slint/native screenshot against .screenshots/main-desktop.png.
Images must have identical pixel dimensions; recapture the native window at the same nominal size if they differ.

Options:
  --baseline <path>    Reference PNG. Defaults to .screenshots/main-desktop.png
  --output <path>      Diff PNG path. Defaults to prototypes/slint-kalpa/tools/diff-output/<candidate>.diff.png
  --threshold <0-255>  Per-channel delta threshold for changed pixels. Defaults to 12
  --json              Print metrics as JSON
  --help              Show this help
`);
}

function parseArgs(argv) {
  const args = {
    baseline: defaultBaseline,
    candidate: undefined,
    output: undefined,
    threshold: 12,
    json: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];

    if (arg === "--help" || arg === "-h") {
      args.help = true;
      continue;
    }

    if (arg === "--json") {
      args.json = true;
      continue;
    }

    if (arg === "--baseline" || arg === "--output" || arg === "--threshold") {
      const value = argv[index + 1];
      if (!value || value.startsWith("--")) {
        throw new Error(`${arg} requires a value.`);
      }

      if (arg === "--baseline") {
        args.baseline = value;
      } else if (arg === "--output") {
        args.output = value;
      } else {
        const threshold = Number(value);
        if (!Number.isFinite(threshold) || threshold < 0 || threshold > 255) {
          throw new Error("--threshold must be a number from 0 to 255.");
        }
        args.threshold = threshold;
      }

      index += 1;
      continue;
    }

    if (arg.startsWith("--")) {
      throw new Error(`Unknown option: ${arg}`);
    }

    if (args.candidate) {
      throw new Error(`Unexpected extra argument: ${arg}`);
    }

    args.candidate = arg;
  }

  return args;
}

function resolveFromCwd(filePath) {
  return path.resolve(process.cwd(), filePath);
}

function formatPercent(value) {
  return `${(value * 100).toFixed(4)}%`;
}

function formatNumber(value) {
  return value.toLocaleString("en-US");
}

function colorTypeName(colorType) {
  switch (colorType) {
    case 0:
      return "grayscale";
    case 2:
      return "RGB";
    case 4:
      return "grayscale+alpha";
    case 6:
      return "RGBA";
    default:
      return `unsupported color type ${colorType}`;
  }
}

function channelsForColorType(colorType) {
  switch (colorType) {
    case 0:
      return 1;
    case 2:
      return 3;
    case 4:
      return 2;
    case 6:
      return 4;
    default:
      throw new Error(`Unsupported PNG color type ${colorType}. Expected an 8-bit RGB or RGBA PNG.`);
  }
}

function paethPredictor(left, above, upperLeft) {
  const p = left + above - upperLeft;
  const pa = Math.abs(p - left);
  const pb = Math.abs(p - above);
  const pc = Math.abs(p - upperLeft);

  if (pa <= pb && pa <= pc) {
    return left;
  }

  if (pb <= pc) {
    return above;
  }

  return upperLeft;
}

function decodePng(buffer, sourceLabel) {
  if (buffer.length < pngSignature.length || !buffer.subarray(0, pngSignature.length).equals(pngSignature)) {
    throw new Error(`${sourceLabel} is not a PNG file.`);
  }

  let offset = pngSignature.length;
  let header;
  const idatChunks = [];

  while (offset < buffer.length) {
    if (offset + 12 > buffer.length) {
      throw new Error(`${sourceLabel} has a truncated PNG chunk.`);
    }

    const length = buffer.readUInt32BE(offset);
    const type = buffer.toString("ascii", offset + 4, offset + 8);
    const dataStart = offset + 8;
    const dataEnd = dataStart + length;

    if (dataEnd + 4 > buffer.length) {
      throw new Error(`${sourceLabel} has a truncated ${type} chunk.`);
    }

    const data = buffer.subarray(dataStart, dataEnd);

    if (type === "IHDR") {
      header = {
        width: data.readUInt32BE(0),
        height: data.readUInt32BE(4),
        bitDepth: data[8],
        colorType: data[9],
        compression: data[10],
        filter: data[11],
        interlace: data[12],
      };
    } else if (type === "IDAT") {
      idatChunks.push(data);
    } else if (type === "IEND") {
      break;
    }

    offset = dataEnd + 4;
  }

  if (!header) {
    throw new Error(`${sourceLabel} is missing a PNG header.`);
  }

  if (header.bitDepth !== 8) {
    throw new Error(`${sourceLabel} uses ${header.bitDepth}-bit PNG data. This harness supports 8-bit PNG screenshots.`);
  }

  if (header.compression !== 0 || header.filter !== 0 || header.interlace !== 0) {
    throw new Error(`${sourceLabel} uses unsupported PNG compression/filter/interlace settings.`);
  }

  const channels = channelsForColorType(header.colorType);
  const stride = header.width * channels;
  const inflated = inflateSync(Buffer.concat(idatChunks));
  const expectedLength = (stride + 1) * header.height;

  if (inflated.length < expectedLength) {
    throw new Error(`${sourceLabel} has truncated image data.`);
  }

  const unfiltered = Buffer.alloc(header.width * header.height * channels);
  let inputOffset = 0;

  for (let y = 0; y < header.height; y += 1) {
    const filterType = inflated[inputOffset];
    inputOffset += 1;

    const rowOffset = y * stride;
    const previousRowOffset = rowOffset - stride;

    for (let x = 0; x < stride; x += 1) {
      const raw = inflated[inputOffset + x];
      const left = x >= channels ? unfiltered[rowOffset + x - channels] : 0;
      const above = y > 0 ? unfiltered[previousRowOffset + x] : 0;
      const upperLeft = y > 0 && x >= channels ? unfiltered[previousRowOffset + x - channels] : 0;

      let value;
      switch (filterType) {
        case 0:
          value = raw;
          break;
        case 1:
          value = raw + left;
          break;
        case 2:
          value = raw + above;
          break;
        case 3:
          value = raw + Math.floor((left + above) / 2);
          break;
        case 4:
          value = raw + paethPredictor(left, above, upperLeft);
          break;
        default:
          throw new Error(`${sourceLabel} uses unsupported PNG row filter ${filterType}.`);
      }

      unfiltered[rowOffset + x] = value & 0xff;
    }

    inputOffset += stride;
  }

  const rgba = Buffer.alloc(header.width * header.height * 4);

  for (let pixel = 0; pixel < header.width * header.height; pixel += 1) {
    const source = pixel * channels;
    const target = pixel * 4;

    if (header.colorType === 0) {
      const gray = unfiltered[source];
      rgba[target] = gray;
      rgba[target + 1] = gray;
      rgba[target + 2] = gray;
      rgba[target + 3] = 255;
    } else if (header.colorType === 2) {
      rgba[target] = unfiltered[source];
      rgba[target + 1] = unfiltered[source + 1];
      rgba[target + 2] = unfiltered[source + 2];
      rgba[target + 3] = 255;
    } else if (header.colorType === 4) {
      const gray = unfiltered[source];
      rgba[target] = gray;
      rgba[target + 1] = gray;
      rgba[target + 2] = gray;
      rgba[target + 3] = unfiltered[source + 1];
    } else {
      rgba[target] = unfiltered[source];
      rgba[target + 1] = unfiltered[source + 1];
      rgba[target + 2] = unfiltered[source + 2];
      rgba[target + 3] = unfiltered[source + 3];
    }
  }

  return {
    width: header.width,
    height: header.height,
    colorType: colorTypeName(header.colorType),
    rgba,
  };
}

let crcTable;

function makeCrcTable() {
  const table = new Uint32Array(256);

  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[n] = c >>> 0;
  }

  return table;
}

function crc32(buffer) {
  crcTable ??= makeCrcTable();
  let c = 0xffffffff;

  for (const byte of buffer) {
    c = crcTable[(c ^ byte) & 0xff] ^ (c >>> 8);
  }

  return (c ^ 0xffffffff) >>> 0;
}

function pngChunk(type, data = Buffer.alloc(0)) {
  const typeBuffer = Buffer.from(type, "ascii");
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);

  const checksumInput = Buffer.concat([typeBuffer, data]);
  const checksum = Buffer.alloc(4);
  checksum.writeUInt32BE(crc32(checksumInput), 0);

  return Buffer.concat([length, typeBuffer, data, checksum]);
}

function encodePng(width, height, rgba) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8;
  header[9] = 6;
  header[10] = 0;
  header[11] = 0;
  header[12] = 0;

  const stride = width * 4;
  const scanlines = Buffer.alloc((stride + 1) * height);

  for (let y = 0; y < height; y += 1) {
    const rowStart = y * (stride + 1);
    scanlines[rowStart] = 0;
    rgba.copy(scanlines, rowStart + 1, y * stride, y * stride + stride);
  }

  return Buffer.concat([
    pngSignature,
    pngChunk("IHDR", header),
    pngChunk("IDAT", deflateSync(scanlines)),
    pngChunk("IEND"),
  ]);
}

function compareImages(baseline, candidate, threshold) {
  const totalPixels = baseline.width * baseline.height;
  const diff = Buffer.alloc(totalPixels * 4);
  let changedPixels = 0;
  let totalAbsoluteChannelDelta = 0;
  let totalSquaredChannelDelta = 0;
  let maxChannelDelta = 0;

  for (let pixel = 0; pixel < totalPixels; pixel += 1) {
    const offset = pixel * 4;
    const redDelta = Math.abs(baseline.rgba[offset] - candidate.rgba[offset]);
    const greenDelta = Math.abs(baseline.rgba[offset + 1] - candidate.rgba[offset + 1]);
    const blueDelta = Math.abs(baseline.rgba[offset + 2] - candidate.rgba[offset + 2]);
    const alphaDelta = Math.abs(baseline.rgba[offset + 3] - candidate.rgba[offset + 3]);
    const pixelMaxDelta = Math.max(redDelta, greenDelta, blueDelta, alphaDelta);

    totalAbsoluteChannelDelta += redDelta + greenDelta + blueDelta + alphaDelta;
    totalSquaredChannelDelta += redDelta ** 2 + greenDelta ** 2 + blueDelta ** 2 + alphaDelta ** 2;
    maxChannelDelta = Math.max(maxChannelDelta, pixelMaxDelta);

    if (pixelMaxDelta > threshold) {
      changedPixels += 1;
      const intensity = Math.max(96, pixelMaxDelta);
      diff[offset] = 255;
      diff[offset + 1] = Math.max(0, 80 - Math.floor(pixelMaxDelta / 4));
      diff[offset + 2] = intensity;
      diff[offset + 3] = 255;
    } else {
      const gray = Math.round(
        0.2126 * baseline.rgba[offset] +
          0.7152 * baseline.rgba[offset + 1] +
          0.0722 * baseline.rgba[offset + 2],
      );
      const dimmed = Math.round(32 + gray * 0.35);
      diff[offset] = dimmed;
      diff[offset + 1] = dimmed;
      diff[offset + 2] = dimmed;
      diff[offset + 3] = 255;
    }
  }

  const channelCount = totalPixels * 4;

  return {
    diff,
    metrics: {
      width: baseline.width,
      height: baseline.height,
      totalPixels,
      changedPixels,
      changedRatio: changedPixels / totalPixels,
      meanAbsoluteChannelDelta: totalAbsoluteChannelDelta / channelCount,
      rootMeanSquaredChannelDelta: Math.sqrt(totalSquaredChannelDelta / channelCount),
      maxChannelDelta,
      threshold,
    },
  };
}

async function readPng(filePath, label) {
  const buffer = await readFile(filePath);
  return decodePng(buffer, label);
}

function printTextReport(report) {
  console.log(`Baseline:  ${report.baseline.path}`);
  console.log(`Candidate: ${report.candidate.path}`);
  console.log(`Dimensions: baseline ${report.baseline.width}x${report.baseline.height} (${report.baseline.colorType}), candidate ${report.candidate.width}x${report.candidate.height} (${report.candidate.colorType})`);
  console.log(`Changed pixels: ${formatNumber(report.metrics.changedPixels)} / ${formatNumber(report.metrics.totalPixels)} (${formatPercent(report.metrics.changedRatio)}) using threshold ${report.metrics.threshold}`);
  console.log(`Mean abs channel delta: ${report.metrics.meanAbsoluteChannelDelta.toFixed(3)}`);
  console.log(`RMSE channel delta: ${report.metrics.rootMeanSquaredChannelDelta.toFixed(3)}`);
  console.log(`Max channel delta: ${report.metrics.maxChannelDelta}`);
  console.log(`Diff image: ${report.diffPath}`);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));

  if (args.help) {
    usage();
    return;
  }

  if (!args.candidate) {
    usage();
    process.exitCode = 2;
    return;
  }

  const baselinePath = resolveFromCwd(args.baseline);
  const candidatePath = resolveFromCwd(args.candidate);
  const candidateName = path.basename(candidatePath, path.extname(candidatePath));
  const outputPath = resolveFromCwd(args.output ?? path.join(defaultOutputDir, `${candidateName}.diff.png`));

  if (!existsSync(baselinePath)) {
    throw new Error(`Baseline screenshot not found: ${baselinePath}`);
  }

  if (!existsSync(candidatePath)) {
    throw new Error(`Candidate screenshot not found: ${candidatePath}`);
  }

  const baseline = await readPng(baselinePath, "baseline");
  const candidate = await readPng(candidatePath, "candidate");

  if (baseline.width !== candidate.width || baseline.height !== candidate.height) {
    const mismatchReport = {
      baseline: {
        path: baselinePath,
        width: baseline.width,
        height: baseline.height,
        colorType: baseline.colorType,
      },
      candidate: {
        path: candidatePath,
        width: candidate.width,
        height: candidate.height,
        colorType: candidate.colorType,
      },
    };

    if (args.json) {
      console.log(JSON.stringify({ ok: false, error: "dimension-mismatch", ...mismatchReport }, null, 2));
    } else {
      console.log(`Baseline:  ${baselinePath}`);
      console.log(`Candidate: ${candidatePath}`);
      console.log(`Dimensions: baseline ${baseline.width}x${baseline.height} (${baseline.colorType}), candidate ${candidate.width}x${candidate.height} (${candidate.colorType})`);
      console.error("Images must have identical dimensions. Recapture the native prototype at the same nominal window size before diffing.");
    }

    process.exitCode = 1;
    return;
  }

  const { diff, metrics } = compareImages(baseline, candidate, args.threshold);
  await mkdir(path.dirname(outputPath), { recursive: true });
  await writeFile(outputPath, encodePng(baseline.width, baseline.height, diff));

  const report = {
    ok: true,
    baseline: {
      path: baselinePath,
      width: baseline.width,
      height: baseline.height,
      colorType: baseline.colorType,
    },
    candidate: {
      path: candidatePath,
      width: candidate.width,
      height: candidate.height,
      colorType: candidate.colorType,
    },
    metrics,
    diffPath: outputPath,
  };

  if (args.json) {
    console.log(JSON.stringify(report, null, 2));
  } else {
    printTextReport(report);
  }
}

main().catch((error) => {
  console.error(`screenshot-diff: ${error.message}`);
  process.exitCode = 1;
});
