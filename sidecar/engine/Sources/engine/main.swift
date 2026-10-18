import AVFoundation
import CoreGraphics
import CryptoKit
import Foundation
import ImageIO
import UniformTypeIdentifiers

// fotoflow engine — JSON-lines over stdio (protocol v1).
//
// Request:   {"id": N, "cmd": "...", "params": {...}}
// Progress:  {"event": "progress", "id": N, "data": {...}}
// Chunk:     {"event": "chunk", "id": N, "data": [...]}
// Response:  {"id": N, "ok": true, "result": {...}}
//            {"id": N, "ok": false, "error": "..."}

struct EngineError: Error { let message: String }

let CHUNK_SIZE = 200

func makeResponse(id: Int, result: Any? = nil, error: String? = nil) -> Data {
    var obj: [String: Any] = ["id": id]
    if let error {
        obj["ok"] = false
        obj["error"] = error
    } else {
        obj["ok"] = true
        obj["result"] = result ?? [String: Any]()
    }
    return try! JSONSerialization.data(withJSONObject: obj)
}

func makeEvent(id: Int, event: String, data: Any) -> Data {
    let obj: [String: Any] = ["event": event, "id": id, "data": data]
    return try! JSONSerialization.data(withJSONObject: obj)
}

func write(_ data: Data) {
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data("\n".utf8))
}

func unameArch() -> String {
    var uts = utsname()
    if uname(&uts) != 0 { return "unknown" }
    var machine = uts.machine
    let capacity = MemoryLayout.size(ofValue: machine)
    return withUnsafePointer(to: &machine) {
        $0.withMemoryRebound(to: CChar.self, capacity: capacity) {
            String(validatingUTF8: $0) ?? "unknown"
        }
    }
}

func sysInfo() -> [String: Any] {
    let proc = ProcessInfo.processInfo
    return [
        "os": proc.operatingSystemVersionString,
        "arch": unameArch(),
        "cpuCount": proc.activeProcessorCount,
        "memoryGB": Double(proc.physicalMemory) / 1_000_000_000.0,
        "swift": "6.2",
        "protocol": 1,
    ]
}

// MARK: - Filesystem

let IMAGE_EXTS: Set<String> = [
    "jpg", "jpeg", "heic", "heif", "png", "tif", "tiff", "gif", "webp", "bmp", "dng", "raw", "cr2",
    "cr3", "nef", "arw", "orf", "rw2", "raf", "avif",
]
let VIDEO_EXTS: Set<String> = [
    "mov", "mp4", "m4v", "avi", "mpg", "mpeg", "hevc", "mkv", "webm", "3gp", "mts", "m2ts",
]

func kindOf(ext: String) -> String {
    if IMAGE_EXTS.contains(ext) { return "image" }
    if VIDEO_EXTS.contains(ext) { return "video" }
    return "other"
}

func mimeOf(path: String, kind: String) -> String {
    if let type = UTType(filenameExtension: (path as NSString).pathExtension), let mime = type.preferredMIMEType {
        return mime
    }
    return kind == "image" ? "image/jpeg" : "application/octet-stream"
}

struct AssetEntry {
    var path: String
    var name: String
    var ext: String
    var kind: String
    var sizeBytes: Int64
    var modified: Double
    var mime: String

    func json() -> [String: Any] {
        [
            "path": path, "name": name, "ext": ext, "kind": kind,
            "sizeBytes": sizeBytes, "modified": modified, "mime": mime,
        ]
    }
}

func scan(root: String, recursive: Bool, includeHidden: Bool) throws -> (count: Int, skipped: Int) {
    let fm = FileManager.default
    let rootURL = URL(fileURLWithPath: root)
    var isDir: ObjCBool = false
    guard fm.fileExists(atPath: root, isDirectory: &isDir), isDir.boolValue else {
        throw EngineError(message: "not a directory: \(root)")
    }

    var options: FileManager.DirectoryEnumerationOptions = [.skipsPackageDescendants]
    if !includeHidden { options.insert(.skipsHiddenFiles) }
    if !recursive { options.insert(.skipsSubdirectoryDescendants) }

    let keys: [URLResourceKey] = [.isDirectoryKey, .fileSizeKey, .contentModificationDateKey]
    guard
        let walker = fm.enumerator(
            at: rootURL, includingPropertiesForKeys: keys, options: options,
            errorHandler: { _, _ in true })
    else {
        throw EngineError(message: "cannot enumerate \(root)")
    }

    var batch: [Any] = []
    var count = 0
    var skipped = 0
    var bytes: Int64 = 0

    func flush() {
        guard !batch.isEmpty else { return }
        write(makeEvent(id: requestID, event: "chunk", data: batch))
        batch.removeAll(keepingCapacity: true)
    }

    for case let fileURL as URL in walker {
        let values = try? fileURL.resourceValues(forKeys: Set(keys))
        if values?.isDirectory == true { continue }

        let ext = fileURL.pathExtension.lowercased()
        let kind = kindOf(ext: ext)
        if kind == "other" { skipped += 1; continue }

        let attrs = try? fm.attributesOfItem(atPath: fileURL.path)
        let size = (attrs?[.size] as? NSNumber)?.int64Value ?? Int64(values?.fileSize ?? 0)
        let modified = (attrs?[.modificationDate] as? Date)?.timeIntervalSince1970 ?? 0

        let entry = AssetEntry(
            path: fileURL.path, name: fileURL.lastPathComponent, ext: ext, kind: kind,
            sizeBytes: size, modified: modified, mime: mimeOf(path: fileURL.path, kind: kind)
        )
        batch.append(entry.json())
        count += 1
        bytes += size

        if batch.count >= CHUNK_SIZE {
            flush()
            write(makeEvent(id: requestID, event: "progress", data: [
                "scanned": count, "bytes": bytes, "kind": kind,
            ]))
        }
    }
    flush()

    return (count, skipped)
}

// MARK: - Media metadata

/// EXIF timestamps are local wall-clock without a timezone suffix.
func parseExifDate(_ s: String) -> Double? {
    let fmt = DateFormatter()
    fmt.dateFormat = "yyyy:MM:dd HH:mm:ss"
    fmt.locale = Locale(identifier: "en_US_POSIX")
    return fmt.date(from: s)?.timeIntervalSince1970
}

func imageStat(path: String) -> [String: Any] {
    var out: [String: Any] = ["path": path, "kind": "image"]
    let url = URL(fileURLWithPath: path)
    guard
        let src = CGImageSourceCreateWithURL(url as CFURL, [
            kCGImageSourceShouldCache: false,
        ] as CFDictionary)
    else {
        out["error"] = "unreadable image"
        return out
    }

    let props = CGImageSourceCopyPropertiesAtIndex(src, 0, nil) as? [CFString: Any] ?? [:]
    if let w = (props[kCGImagePropertyPixelWidth] as? NSNumber)?.intValue { out["width"] = w }
    if let h = (props[kCGImagePropertyPixelHeight] as? NSNumber)?.intValue { out["height"] = h }
    if let o = (props[kCGImagePropertyOrientation] as? NSNumber)?.intValue { out["orientation"] = o }
    let frames = CGImageSourceGetCount(src)
    if frames > 1 { out["frameCount"] = frames }
    if let icc = props[kCGImagePropertyProfileName] as? String { out["profile"] = icc }

    let exif = props[kCGImagePropertyExifDictionary] as? [CFString: Any] ?? [:]
    let tiff = props[kCGImagePropertyTIFFDictionary] as? [CFString: Any] ?? [:]
    let gps = props[kCGImagePropertyGPSDictionary] as? [CFString: Any] ?? [:]

    if let s = tiff[kCGImagePropertyTIFFDateTime] as? String, let t = parseExifDate(s) {
        out["capturedAt"] = t
    } else if let s = exif[kCGImagePropertyExifDateTimeOriginal] as? String, let t = parseExifDate(s) {
        out["capturedAt"] = t
    }
    if let make = tiff[kCGImagePropertyTIFFMake] as? String { out["cameraMake"] = make }
    if let model = tiff[kCGImagePropertyTIFFModel] as? String { out["cameraModel"] = model }
    if let lens = exif[kCGImagePropertyExifLensModel] as? String { out["lens"] = lens }
    if let iso = (exif[kCGImagePropertyExifISOSpeedRatings] as? [NSNumber])?.first?.intValue {
        out["iso"] = iso
    }
    if let fnum = exif[kCGImagePropertyExifFNumber] as? NSNumber { out["fNumber"] = fnum.doubleValue }
    if let exp = exif[kCGImagePropertyExifExposureTime] as? NSNumber { out["exposureTime"] = exp.doubleValue }
    if let lat = gps[kCGImagePropertyGPSLatitude] as? NSNumber {
        let ref = (gps[kCGImagePropertyGPSLatitudeRef] as? String) ?? "N"
        out["latitude"] = lat.doubleValue * (ref == "S" ? -1 : 1)
    }
    if let lon = gps[kCGImagePropertyGPSLongitude] as? NSNumber {
        let ref = (gps[kCGImagePropertyGPSLongitudeRef] as? String) ?? "E"
        out["longitude"] = lon.doubleValue * (ref == "W" ? -1 : 1)
    }
    return out
}

func videoStat(path: String) -> [String: Any] {
    var out: [String: Any] = ["path": path, "kind": "video"]
    let asset = AVURLAsset(url: URL(fileURLWithPath: path))
    let seconds = CMTimeGetSeconds(asset.duration)
    if seconds.isFinite, seconds > 0 { out["duration"] = seconds }
    if let track = asset.tracks(withMediaType: .video).first {
        let size = track.naturalSize.applying(track.preferredTransform)
        out["width"] = Int(abs(size.width).rounded())
        out["height"] = Int(abs(size.height).rounded())
        let fps = track.nominalFrameRate
        if fps.isFinite, fps > 0 { out["fps"] = fps }
    }
    if let item = asset.creationDate, let s = item.stringValue, let t = parseISO8601(s) {
        out["capturedAt"] = t
    }
    return out
}

/// AVMetadataItem creation dates are ISO-8601, unlike EXIF's colon-separated form.
func parseISO8601(_ s: String) -> Double? {
    let fmt = ISO8601DateFormatter()
    fmt.formatOptions = [.withInternetDateTime]
    if let d = fmt.date(from: s) { return d.timeIntervalSince1970 }
    // Bare local timestamps (no zone suffix) appear in some QuickTime files.
    let local = DateFormatter()
    local.dateFormat = "yyyy-MM-dd'T'HH:mm:ss"
    local.locale = Locale(identifier: "en_US_POSIX")
    return local.date(from: s)?.timeIntervalSince1970
}

func stat(path: String) throws -> [String: Any] {
    guard FileManager.default.fileExists(atPath: path) else {
        throw EngineError(message: "no such file: \(path)")
    }
    let ext = (path as NSString).pathExtension.lowercased()
    let kind = kindOf(ext: ext)
    var out: [String: Any]
    switch kind {
    case "image": out = imageStat(path: path)
    case "video": out = videoStat(path: path)
    default: throw EngineError(message: "unsupported kind for stat: \(ext)")
    }
    let attrs = try? FileManager.default.attributesOfItem(atPath: path)
    out["sizeBytes"] = (attrs?[.size] as? NSNumber)?.int64Value ?? 0
    out["modified"] = (attrs?[.modificationDate] as? Date)?.timeIntervalSince1970 ?? 0
    out["mime"] = mimeOf(path: path, kind: kind)
    return out
}

// MARK: - Hashing

func sha256File(path: String) throws -> String {
    let handle = try FileHandle(forReadingFrom: URL(fileURLWithPath: path))
    defer { try? handle.close() }
    var hasher = SHA256()
    while let data = try handle.read(upToCount: 1 << 20), !data.isEmpty {
        hasher.update(data: data)
    }
    return hasher.finalize().map { String(format: "%02x", $0) }.joined()
}

func hashAsset(path: String) throws -> [String: Any] {
    let started = Date()
    let digest = try sha256File(path: path)
    let attrs = try FileManager.default.attributesOfItem(atPath: path)
    return [
        "path": path,
        "sha256": digest,
        "sizeBytes": (attrs[.size] as? NSNumber)?.int64Value ?? 0,
        "elapsed": Date().timeIntervalSince(started),
    ]
}

// MARK: - Thumbnails

func thumbnail(path: String, maxPx: Int, quality: Double) throws -> [String: Any] {
    let ext = (path as NSString).pathExtension.lowercased()
    let px = max(16, min(maxPx, 4096))
    let out: [String: Any]

    if VIDEO_EXTS.contains(ext) {
        let asset = AVURLAsset(url: URL(fileURLWithPath: path))
        let gen = AVAssetImageGenerator(asset: asset)
        gen.appliesPreferredTrackTransform = true
        gen.maximumSize = CGSize(width: px, height: px)
        let time = CMTime(seconds: 0.5, preferredTimescale: 600)
        guard let cg = try? gen.copyCGImage(at: time, actualTime: nil) else {
            throw EngineError(message: "cannot decode video frame: \(path)")
        }
        out = ["path": path, "image": encodeJPEG(cg, quality: quality)]
    } else {
        let src = CGImageSourceCreateWithURL(
            URL(fileURLWithPath: path) as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary)
        guard let src else { throw EngineError(message: "unreadable image: \(path)") }
        let opts: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: px,
        ]
        guard let cg = CGImageSourceCreateThumbnailAtIndex(src, 0, opts as CFDictionary) else {
            throw EngineError(message: "cannot decode image: \(path)")
        }
        out = ["path": path, "image": encodeJPEG(cg, quality: quality)]
    }

    return out.merging(["mime": "image/jpeg", "maxPx": px]) { _, new in new }
}

func encodeJPEG(_ image: CGImage, quality: Double) -> String {
    let data = NSMutableData()
    guard
        let dest = CGImageDestinationCreateWithData(
            data as CFMutableData, "public.jpeg" as CFString, 1, nil)
    else { return "" }
    CGImageDestinationAddImage(
        dest, image, [kCGImageDestinationLossyCompressionQuality: min(max(quality, 0.1), 1.0)] as CFDictionary
    )
    guard CGImageDestinationFinalize(dest) else { return "" }
    return (data as Data).base64EncodedString()
}

// MARK: - Dispatch

/// Set by the read loop so streaming commands can tag their events.
var requestID: Int = -1

func handle(cmd: String, params: [String: Any]) throws -> [String: Any] {
    func str(_ key: String) -> String? { params[key] as? String }
    func flag(_ key: String, _ fallback: Bool) -> Bool {
        if let b = params[key] as? Bool { return b }
        return fallback
    }
    func int(_ key: String, _ fallback: Int) -> Int {
        if let n = params[key] as? NSNumber { return n.intValue }
        return fallback
    }
    func dbl(_ key: String, _ fallback: Double) -> Double {
        if let n = params[key] as? NSNumber { return n.doubleValue }
        return fallback
    }

    switch cmd {
    case "ping":
        return ["pong": true]
    case "sysinfo":
        return sysInfo()

    case "scan":
        guard let root = str("root") else { throw EngineError(message: "scan requires 'root'") }
        let started = Date()
        let (count, skipped) = try scan(
            root: root, recursive: flag("recursive", true), includeHidden: flag("includeHidden", false))
        return [
            "root": root,
            "scanned": count,
            "skipped": skipped,
            "elapsed": Date().timeIntervalSince(started),
        ]

    case "stat":
        guard let path = str("path") else { throw EngineError(message: "stat requires 'path'") }
        return try stat(path: path)

    case "hash":
        guard let path = str("path") else { throw EngineError(message: "hash requires 'path'") }
        return try hashAsset(path: path)

    case "thumb":
        guard let path = str("path") else { throw EngineError(message: "thumb requires 'path'") }
        return try thumbnail(
            path: path, maxPx: int("maxPx", 512), quality: dbl("quality", 0.72))

    default:
        throw EngineError(message: "unknown command '\(cmd)'")
    }
}

while let line = readLine() {
    let trimmed = line.trimmingCharacters(in: .whitespaces)
    if trimmed.isEmpty { continue }
    guard let data = trimmed.data(using: .utf8),
          let req = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let id = req["id"] as? Int else {
        write(makeResponse(id: -1, error: "malformed request"))
        continue
    }
    let cmd = req["cmd"] as? String ?? ""
    let params = req["params"] as? [String: Any] ?? [:]
    requestID = id
    do {
        let result = try handle(cmd: cmd, params: params)
        write(makeResponse(id: id, result: result))
    } catch let err as EngineError {
        write(makeResponse(id: id, error: err.message))
    } catch {
        write(makeResponse(id: id, error: String(describing: error)))
    }
}
