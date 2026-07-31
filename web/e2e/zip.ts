import { crc32 } from 'node:zlib';

/**
 * Builds a stored (uncompressed) ZIP archive from `(path, bytes)` pairs.
 *
 * Hand-assembled rather than pulled from a dependency: the format's stored
 * variant is a page of code, and the suite should not grow an archive library
 * to produce a three-file fixture. Every offset and CRC is real, so any
 * compliant reader — including the server's — accepts the result.
 */
export function makeZip(files: readonly (readonly [string, Buffer])[]): Buffer {
  const chunks: Buffer[] = [];
  const central: Buffer[] = [];
  let offset = 0;

  for (const [name, data] of files) {
    const nameBytes = Buffer.from(name, 'utf8');
    const checksum = crc32(data);

    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04_03_4b_50, 0); // local file header signature
    local.writeUInt16LE(20, 4); // version needed
    local.writeUInt16LE(0, 6); // flags
    local.writeUInt16LE(0, 8); // method: stored
    local.writeUInt32LE(0, 10); // dos time and date
    local.writeUInt32LE(checksum, 14);
    local.writeUInt32LE(data.length, 18); // compressed size
    local.writeUInt32LE(data.length, 22); // uncompressed size
    local.writeUInt16LE(nameBytes.length, 26);
    local.writeUInt16LE(0, 28); // extra length
    chunks.push(local, nameBytes, data);

    const entry = Buffer.alloc(46);
    entry.writeUInt32LE(0x02_01_4b_50, 0); // central directory signature
    entry.writeUInt16LE(20, 4); // version made by
    entry.writeUInt16LE(20, 6); // version needed
    entry.writeUInt16LE(0, 8); // flags
    entry.writeUInt16LE(0, 10); // method: stored
    entry.writeUInt32LE(0, 12); // dos time and date
    entry.writeUInt32LE(checksum, 16);
    entry.writeUInt32LE(data.length, 20);
    entry.writeUInt32LE(data.length, 24);
    entry.writeUInt16LE(nameBytes.length, 28);
    // extra, comment, disk, internal and external attributes: all zero.
    entry.writeUInt32LE(offset, 42); // local header offset
    central.push(Buffer.concat([entry, nameBytes]));

    offset += local.length + nameBytes.length + data.length;
  }

  const directory = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06_05_4b_50, 0); // end of central directory signature
  end.writeUInt16LE(files.length, 8); // entries on this disk
  end.writeUInt16LE(files.length, 10); // entries total
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16); // directory offset
  chunks.push(directory, end);

  return Buffer.concat(chunks);
}
