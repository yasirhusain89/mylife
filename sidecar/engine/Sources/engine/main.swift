import AppKit
import AudioToolbox
import AVFoundation
import Compression
import CoreGraphics
import CoreImage
import CoreML
import CryptoKit
import Foundation
import ImageIO
import UniformTypeIdentifiers
import Vision
import VisionKit

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

func imageStat(path: String) throws -> [String: Any] {
    var out: [String: Any] = ["path": path, "kind": "image"]
    let url = URL(fileURLWithPath: path)
    guard
        let src = CGImageSourceCreateWithURL(url as CFURL, [
            kCGImageSourceShouldCache: false,
        ] as CFDictionary)
    else {
        throw EngineError(message: "not an image container: \(path)")
    }

    // Creating a source succeeds for arbitrary bytes; the verdict is in the
    // status. A complete source is trustworthy without paying for a full decode.
    let status = CGImageSourceGetStatus(src)
    if status != .statusComplete {
        // statusUnknown covers formats ImageIO probes lazily, so fall back to an
        // actual decode before declaring the file unreadable.
        guard CGImageSourceCreateImageAtIndex(src, 0, nil) != nil else {
            // Must fail the call rather than embed an error key: callers use this
            // outcome to tell "permanently corrupt" from "not downloaded yet".
            throw EngineError(message: "unreadable image (\(status)): \(path)")
        }
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
    if let focal = exif[kCGImagePropertyExifFocalLength] as? NSNumber { out["focalLength"] = focal.doubleValue }
    if let focal35 = exif[kCGImagePropertyExifFocalLenIn35mmFilm] as? NSNumber { out["focalLength35"] = focal35.intValue }
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
    // Capture date, strongest signal first: the native date value, then a
    // scan of the QuickTime/common metadata items (creationdate strings need
    // both ISO-8601 and EXIF-colon parses), then the camera filename stamp.
    // Videos previously enriched with none of these stay dateless and keep
    // sorting by file mtime — the filename fallback recovers those.
    if let d = asset.creationDate?.dateValue {
        out["capturedAt"] = d.timeIntervalSince1970
    } else {
        for item in asset.metadata {
            if out["capturedAt"] == nil {
                // commonKeyCreationDate's raw value ("creationDate"); the
                // QuickTime/mdta identifiers are matched as raw strings so no
                // SDK-version-sensitive static members are referenced.
                let isDateKey = item.commonKey?.rawValue == "creationDate"
                    || item.identifier?.rawValue == "com.apple.quicktime.creationdate"
                    || item.identifier?.rawValue == "mdta/com.apple.quicktime.creationdate"
                if isDateKey {
                    if let d = item.dateValue {
                        out["capturedAt"] = d.timeIntervalSince1970
                    } else if let s = item.stringValue,
                              let t = parseISO8601(s) ?? parseExifDate(s) {
                        out["capturedAt"] = t
                    }
                }
            }
            if out["latitude"] == nil {
                let isLocKey = item.identifier?.rawValue == "com.apple.quicktime.location.ISO6709"
                    || item.identifier?.rawValue == "mdta/com.apple.quicktime.location.ISO6709"
                if isLocKey, let s = item.stringValue, let (la, lo) = parseISO6709(s) {
                    out["latitude"] = la
                    out["longitude"] = lo
                }
            }
            if out["capturedAt"] != nil && out["latitude"] != nil { break }
        }
        if out["capturedAt"] == nil,
           let t = parseFilenameDate(URL(fileURLWithPath: path).lastPathComponent) {
            out["capturedAt"] = t
        }
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

/// QuickTime location strings (`com.apple.quicktime.location.ISO6709`):
/// decimal degrees like "+38.5724-109.5509/" with an optional altitude
/// segment ("+38.5724-109.5509+1234.0/"). Returns (latitude, longitude).
func parseISO6709(_ s: String) -> (Double, Double)? {
    var t = s.trimmingCharacters(in: .whitespacesAndNewlines)
    if t.hasSuffix("/") { t = String(t.dropLast()) }
    // Split into signed number segments: [+38.5724, -109.5509, +1234.0].
    var nums: [Double] = []
    var cur = ""
    for c in t {
        if c == "+" || c == "-" {
            if let v = Double(cur) { nums.append(v) }
            cur = String(c)
        } else {
            cur.append(c)
        }
    }
    if let v = Double(cur) { nums.append(v) }
    guard nums.count >= 2 else { return nil }
    let (la, lo) = (nums[0], nums[1])
    guard la >= -90, la <= 90, lo >= -180, lo <= 180 else { return nil }
    return (la, lo)
}

/// Camera/phone filename timestamps (IMG_20260419_142159, VID_20261007_192001,
/// video_20260418_144808, PXL_20260419_142159000): YYYYMMDD + HHMMSS, local
/// wall-clock like EXIF. Last resort when the container carries no timestamp
/// (shared/pooled files whose EXIF was stripped).
func parseFilenameDate(_ name: String) -> Double? {
    let stem = (name as NSString).deletingPathExtension
    let chars = Array(stem)
    var i = 0
    while i < chars.count {
        guard chars[i].isNumber else { i += 1; continue }
        var j = i
        while j < chars.count && chars[j].isNumber { j += 1 }
        if j - i == 8 {
            var k = j
            if k < chars.count && (chars[k] == "_" || chars[k] == "-" || chars[k] == " ") { k += 1 }
            if k + 6 <= chars.count && chars[k..<(k + 6)].allSatisfy({ $0.isNumber }) {
                let datePart = String(chars[i..<j])
                let timePart = String(chars[k..<(k + 6)])
                let y = Int(datePart.prefix(4)) ?? 0
                let mo = Int(datePart.dropFirst(4).prefix(2)) ?? 0
                let d = Int(datePart.dropFirst(6).prefix(2)) ?? 0
                let hh = Int(timePart.prefix(2)) ?? 0
                let mm = Int(timePart.dropFirst(2).prefix(2)) ?? 0
                let ss = Int(timePart.dropFirst(4).prefix(2)) ?? 0
                var comps = DateComponents()
                comps.year = y; comps.month = mo; comps.day = d
                comps.hour = hh; comps.minute = mm; comps.second = ss
                if y >= 1995 && y <= 2100 && mo >= 1 && mo <= 12 && d >= 1 && d <= 31
                    && hh <= 23 && mm <= 59 && ss <= 59,
                   let date = Calendar.current.date(from: comps) {
                    return date.timeIntervalSince1970
                }
            }
        }
        i = j
    }
    return nil
}

func stat(path: String) throws -> [String: Any] {
    guard FileManager.default.fileExists(atPath: path) else {
        throw EngineError(message: "no such file: \(path)")
    }
    let ext = (path as NSString).pathExtension.lowercased()
    let kind = kindOf(ext: ext)
    var out: [String: Any]
    switch kind {
    case "image": out = try imageStat(path: path)
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


// MARK: - Faces

/// Detects faces (bounding boxes + confidence) — no descriptors, cheap, works on any image.
func detectFaces(path: String) throws -> [String: Any] {
    let url = URL(fileURLWithPath: path)
    guard FileManager.default.fileExists(atPath: path) else {
        throw EngineError(message: "file not found (volume unmounted?): \(path)")
    }
    guard
        let src = CGImageSourceCreateWithURL(
            url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary)
    else { throw EngineError(message: "not an image: \(path)") }
    guard let cg = CGImageSourceCreateImageAtIndex(src, 0, nil) else {
        throw EngineError(message: "cannot decode image: \(path)")
    }

    let rectRequest = VNDetectFaceRectanglesRequest()
    let handler = VNImageRequestHandler(cgImage: cg, options: [:])
    try handler.perform([rectRequest])

    var faces: [[String: Any]] = []
    for (index, obs) in (rectRequest.results ?? []).enumerated() {
        let b = obs.boundingBox
        faces.append([
            "index": index,
            "x": Double(b.origin.x),
            "y": Double(1.0 - b.origin.y - b.height),
            "width": Double(b.width),
            "height": Double(b.height),
            "confidence": Double(obs.confidence),
        ])
    }
    return ["path": path, "count": faces.count, "faces": faces]
}

/// Pet detection (dogs and cats — the only animals VNRecognizeAnimalsRequest
/// supports per the macOS SDK headers). Same box convention as faces so the
/// UI reuses the crop pipeline. Birds and other animals are NOT detectable
/// with system frameworks; they stay tag-level via `classify`.
func detectAnimals(path: String) throws -> [String: Any] {
    let url = URL(fileURLWithPath: path)
    guard FileManager.default.fileExists(atPath: path) else {
        throw EngineError(message: "file not found (volume unmounted?): \(path)")
    }
    guard
        let src = CGImageSourceCreateWithURL(
            url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary)
    else { throw EngineError(message: "not an image: \(path)") }
    guard let cg = CGImageSourceCreateImageAtIndex(src, 0, nil) else {
        throw EngineError(message: "cannot decode image: \(path)")
    }

    let req = VNRecognizeAnimalsRequest()
    do {
        try VNImageRequestHandler(cgImage: cg, options: [:]).perform([req])
    } catch {
        throw EngineError(message: "animal detection failed: \(error)")
    }

    var animals: [[String: Any]] = []
    for obs in req.results ?? [] {
        guard let label = obs.labels.first?.identifier else { continue }
        let b = obs.boundingBox
        animals.append([
            "label": label,
            "x": Double(b.origin.x),
            "y": Double(1.0 - b.origin.y - b.height),
            "width": Double(b.width),
            "height": Double(b.height),
            "confidence": Double(obs.confidence),
        ])
    }
    return ["path": path, "count": animals.count, "animals": animals]
}

/// Clusters faces across a set of images into "people" using Vision's own
/// feature-print distance. Needs no downloaded model weights: VNGenerateImageFeaturePrintRequest
/// is part of the OS. Returns one cluster per detected person; singletons are
/// groups of size 1.
func clusterFaces(paths: [String], threshold: Double) throws -> [String: Any] {
    struct Face { var path: String; var box: CGRect; var confidence: Double; var desc: FeaturePrintObservation; var descriptorB64: String }
    var entries: [Face] = []
    var unreadable = 0
    var descriptorFailures = 0

    for path in paths {
        let url = URL(fileURLWithPath: path)
        guard
            let src = CGImageSourceCreateWithURL(
                url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
            let cg = CGImageSourceCreateImageAtIndex(src, 0, nil)
        else {
            unreadable += 1
            continue
        }

        let rectRequest = VNDetectFaceRectanglesRequest()
        let rectHandler = VNImageRequestHandler(cgImage: cg, options: [:])
        try rectHandler.perform([rectRequest])
        guard let observations = rectRequest.results, !observations.isEmpty else { continue }

        for obs in observations {
            let b = obs.boundingBox
            let padded = b.insetBy(dx: -b.width * 0.2, dy: -b.height * 0.2)
                .intersection(CGRect(x: 0, y: 0, width: 1, height: 1))
            let fpRequest = VNGenerateImageFeaturePrintRequest()
            fpRequest.regionOfInterest = padded
            let fpHandler = VNImageRequestHandler(cgImage: cg, options: [:])
            guard (try? fpHandler.perform([fpRequest])) != nil,
                  let vnObs = fpRequest.results?.first as? VNFeaturePrintObservation
            else {
                descriptorFailures += 1
                continue
            }
            // VNFeaturePrintObservation (ObjC) and FeaturePrintObservation
            // (Swift struct) are distinct types; results yields the former and
            // distance(to:) lives on the latter.
            let desc = FeaturePrintObservation(vnObs)
            // Persist the feature-print descriptor for Rust-side global
            // regrouping: base64 little-endian f32, same wire format as
            // `embedImage` (doubles normalized to f32).
            let raw = vnObs.data
            let dim = vnObs.elementCount
            let descriptorB64: String
            if vnObs.elementType == .double {
                let count = raw.count / 8
                var floats = [Float](repeating: 0, count: count)
                raw.withUnsafeBytes { ptr in
                    let doubles = ptr.bindMemory(to: Double.self)
                    for i in 0..<count { floats[i] = Float(doubles[i]) }
                }
                descriptorB64 = floats.withUnsafeBytes { Data($0) }.base64EncodedString()
            } else if vnObs.elementType == .float, raw.count == dim * 4 {
                descriptorB64 = raw.base64EncodedString()
            } else {
                descriptorFailures += 1
                continue
            }
            entries.append(Face(path: path, box: b, confidence: Double(obs.confidence), desc: desc, descriptorB64: descriptorB64))
        }
    }

    // Union-find agglomerative clustering on Vision's feature-print distance.
    var parent = Array(0..<entries.count)
    func find(_ i: Int) -> Int {
        if parent[i] != i { parent[i] = find(parent[i]) }
        return parent[i]
    }
    func union(_ a: Int, _ b: Int) {
        let ra = find(a), rb = find(b)
        if ra != rb { parent[rb] = ra }
    }
    for i in 0..<entries.count {
        for j in (i + 1)..<entries.count {
            if let d = try? entries[i].desc.distance(to: entries[j].desc), d < threshold {
                union(i, j)
            }
        }
    }

    var roots: [Int: [Int]] = [:]
    for i in 0..<entries.count { roots[find(i), default: []].append(i) }
    let groups = roots.values.map { members -> [String: Any] in
        [
            "people": members.map { [
                "path": entries[$0].path,
                "x": Double(entries[$0].box.origin.x),
                "y": Double(1.0 - entries[$0].box.origin.y - entries[$0].box.height),
                "width": Double(entries[$0].box.width),
                "height": Double(entries[$0].box.height),
                "confidence": entries[$0].confidence,
                "descriptor": entries[$0].descriptorB64,
            ] },
            "size": members.count,
        ]
    }.sorted { $0["size"] as! Int > $1["size"] as! Int }

    return [
        "faces": entries.count,
        "clusters": groups.count,
        "groups": groups,
        "threshold": threshold,
        "unreadable": unreadable,
        "descriptorFailures": descriptorFailures,
    ]
}


// MARK: - Face thumbnails

/// Crops a detected face region to JPEG for UI thumbnails.
func faceThumbnail(path: String, x: Double, y: Double, width: Double, height: Double, maxPx: Int) throws -> String {
    let url = URL(fileURLWithPath: path)
    guard
        let src = CGImageSourceCreateWithURL(
            url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
        let cg = CGImageSourceCreateImageAtIndex(src, 0, nil)
    else { throw EngineError(message: "cannot decode image: \(path)") }
    let iw = CGFloat(cg.width), ih = CGFloat(cg.height)
    var r = CGRect(x: x * iw, y: y * ih, width: width * iw, height: height * ih)
    r = r.intersection(CGRect(x: 0, y: 0, width: iw, height: ih))
    guard r.width > 0, r.height > 0 else { throw EngineError(message: "empty face crop: \(path)") }
    guard let crop = cg.cropping(to: r) else { throw EngineError(message: "cannot crop face: \(path)") }
    let px = CGFloat(max(32, min(maxPx, 1024)))
    let cw = CGFloat(crop.width), ch = CGFloat(crop.height)
    let scale = min(px / cw, px / ch)
    let size = CGSize(width: CGFloat(crop.width) * scale, height: CGFloat(crop.height) * scale)
    let colorSpace = CGColorSpaceCreateDeviceRGB()
    guard
        let ctx = CGContext(
            data: nil, width: Int(size.width), height: Int(size.height),
            bitsPerComponent: 8, bytesPerRow: 0, space: colorSpace,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    else { throw EngineError(message: "cannot create context: \(path)") }
    ctx.draw(crop, in: CGRect(origin: .zero, size: size))
    guard let rendered = ctx.makeImage() else { throw EngineError(message: "cannot render face crop: \(path)") }
    let data = NSMutableData()
    guard
        let dest = CGImageDestinationCreateWithData(
            data as CFMutableData, "public.jpeg" as CFString, 1, nil)
    else { throw EngineError(message: "cannot create jpeg destination: \(path)") }
    CGImageDestinationAddImage(dest, rendered, [kCGImageDestinationLossyCompressionQuality: 0.72] as CFDictionary)
    guard CGImageDestinationFinalize(dest) else { throw EngineError(message: "cannot finalize face thumbnail: \(path)") }
    return data.base64EncodedString()
}


// MARK: - Highlight video

/// Decodes a still at canvas-cover size plus the Ken Burns zoom headroom
/// (1.10), so every hold frame below blits ~1:1 source pixels instead of
/// resampling the full-res original 90 times. The max-zoom frame uses source
/// pixels 1:1, so output frames match the full-res draw path in coverage —
/// only the wasted oversampling is gone. EXIF orientation is baked in via
/// the thumbnail transform so portrait/rotated stills render as displayed.
/// Falls back to an oriented full-size decode when cover-size creation fails.
func coverThumbnail(_ src: CGImageSource, outW: Int, outH: Int) -> CGImage? {
    let zoomHeadroom = 1.10
    if let props = CGImageSourceCopyPropertiesAtIndex(src, 0, nil) as? [String: Any],
       let w = (props[kCGImagePropertyPixelWidth as String] as? NSNumber)?.doubleValue,
       let h = (props[kCGImagePropertyPixelHeight as String] as? NSNumber)?.doubleValue,
       w > 0, h > 0 {
        let cover = max(Double(outW) / w, Double(outH) / h) * zoomHeadroom
        let maxPx = max(w * cover, h * cover)
        if maxPx < max(w, h),
           let thumb = CGImageSourceCreateThumbnailAtIndex(src, 0, [
               kCGImageSourceShouldCache: false,
               kCGImageSourceCreateThumbnailFromImageAlways: true,
               kCGImageSourceCreateThumbnailWithTransform: true,
               kCGImageSourceThumbnailMaxPixelSize: maxPx as CFNumber,
           ] as CFDictionary) {
            return thumb
        }
        // At or below cover size already, or cover creation failed: oriented
        // full-size decode so EXIF-rotated stills still render as displayed.
        if let thumb = CGImageSourceCreateThumbnailAtIndex(src, 0, [
            kCGImageSourceShouldCache: false,
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: max(w, h) as CFNumber,
        ] as CFDictionary) {
            return thumb
        }
    }
    return CGImageSourceCreateImageAtIndex(src, 0, nil)
}

/// Builds a highlight MP4 from photos and video clips with AVAssetWriter.
///
/// Still images expose zero AVAssetTrack instances on the macOS 26 SDK, so a
/// composition-based export cannot include them; photo frames are synthesized
/// directly instead (3s per photo at 30fps, H.264) with a slow Ken Burns
/// zoom-in and upward drift. Video clips are decoded with AVAssetReader and
/// their frames drawn onto the same 1920x1080 canvas (up to 3s per clip).
/// Video-only: no audio track (camera-audio mixing is deferred — any
/// audio-track interaction poisons video decoding of the same file
/// in-process on macOS 26). Inputs carry optional day-boundary chapter
/// titles, rendered as dark title cards. The timeline is capped at
/// maxSeconds. Files that decode as neither photo nor video count as skipped.
func makeHighlight(inputs: [(path: String, title: String?)], output: String, maxSeconds: Double) throws -> [String: Any] {
    let cap = max(1.0, maxSeconds)

    // Output canvas: 1920x1080 letterboxed (16:9 works for WhatsApp and Drive).
    let outW = 1920, outH = 1080
    let fps = 30
    let audioRate = 44100.0
    let secondsPerPhoto = 3.0
    let secondsPerClip = 3.0
    let framesPerPhoto = Int(secondsPerPhoto * Double(fps))

    // Title cards share the photo bitmap path (row 0 = frame top, as the
    // photo probe proves), so text draws in the context's native space:
    // flipped:false. flipped:true double-flips and stood every caption
    // on its head (OCR read "18 April 2026" as "asoS linqA 8r").
    func titleCardImage(_ text: String) -> CGImage? {
        let colorSpace = CGColorSpaceCreateDeviceRGB()
        guard
            let ctx = CGContext(
                data: nil, width: outW, height: outH, bitsPerComponent: 8,
                bytesPerRow: 0, space: colorSpace,
                bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue
                    | CGBitmapInfo.byteOrder32Little.rawValue)
        else { return nil }
        ctx.setFillColor(CGColor(red: 0.055, green: 0.065, blue: 0.09, alpha: 1.0))
        ctx.fill(CGRect(x: 0, y: 0, width: outW, height: outH))
        let s = text as NSString
        var size: CGFloat = 76
        var attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: size, weight: .semibold),
            .foregroundColor: NSColor(white: 0.96, alpha: 1.0),
        ]
        var textSize = s.size(withAttributes: attrs)
        while textSize.width > CGFloat(outW) - 320 && size > 30 {
            size -= 4
            attrs[.font] = NSFont.systemFont(ofSize: size, weight: .semibold)
            textSize = s.size(withAttributes: attrs)
        }
        let rect = CGRect(
            x: (CGFloat(outW) - textSize.width) / 2,
            y: (CGFloat(outH) - textSize.height) / 2,
            width: textSize.width, height: textSize.height)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: ctx, flipped: false)
        s.draw(in: rect, withAttributes: attrs)
        NSGraphicsContext.restoreGraphicsState()
        return ctx.makeImage()
    }

    enum Clip {
        case photo(CGImage)
        case video(path: String, asset: AVURLAsset, vTrack: AVAssetTrack, hasAudio: Bool)
        case titlecard(CGImage)
    }

    // Presence probe: ExtAudioFile header open only. A missing/unreadable
    // audio stream reads as "no audio" — silence downstream, never an error.
    // Never AVAssetReader audio outputs: those poison in-process video
    // decode on macOS 26.
    func extAudioPresent(_ path: String) -> Bool {
        var fref: ExtAudioFileRef?
        guard ExtAudioFileOpenURL(URL(fileURLWithPath: path) as CFURL, &fref) == noErr,
              let f = fref else { return false }
        defer { ExtAudioFileDispose(f) }
        var client = AudioStreamBasicDescription(mSampleRate: audioRate, mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked, mBytesPerPacket: 4,
            mFramesPerPacket: 1, mBytesPerFrame: 4, mChannelsPerFrame: 1, mBitsPerChannel: 32, mReserved: 0)
        return ExtAudioFileSetProperty(f, kExtAudioFileProperty_ClientDataFormat,
            UInt32(MemoryLayout<AudioStreamBasicDescription>.size), &client) == noErr
    }
    var clips: [Clip] = []
    var skipped = 0
    var prevTitle: String? = nil
    func emitCard(_ title: String?) {
        if let t = title, t != prevTitle {
            if let card = titleCardImage(t) {
                clips.append(.titlecard(card))
            }
            prevTitle = t
        }
    }
    for input in inputs {
        let url = URL(fileURLWithPath: input.path)
        if let src = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
           let cg = coverThumbnail(src, outW: outW, outH: outH) {
            emitCard(input.title)
            clips.append(.photo(cg))
            continue
        }
        let asset = AVURLAsset(url: url)
        if let vTrack = asset.tracks(withMediaType: .video).first {
            emitCard(input.title)
            // Audio presence via ExtAudioFile header open only — never
            // AVAssetReader audio outputs, which poison in-process video
            // decode on macOS 26. Falls back to silence per clip below.
            clips.append(.video(path: input.path, asset: asset, vTrack: vTrack, hasAudio: extAudioPresent(input.path)))
        } else {
            skipped += 1
        }
    }
    guard !clips.isEmpty else { throw EngineError(message: "no decodable photos or videos for highlight") }

    let outURL = URL(fileURLWithPath: output)
    try? FileManager.default.removeItem(atPath: output)
    let writer = try AVAssetWriter(outputURL: outURL, fileType: .mp4)
    let videoSettings: [String: Any] = [
        AVVideoCodecKey: AVVideoCodecType.h264,
        AVVideoWidthKey: outW,
        AVVideoHeightKey: outH,
    ]
    let vInput = AVAssetWriterInput(mediaType: .video, outputSettings: videoSettings)
    vInput.expectsMediaDataInRealTime = false
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(
        assetWriterInput: vInput,
        sourcePixelBufferAttributes: [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
            kCVPixelBufferWidthKey as String: outW,
            kCVPixelBufferHeightKey as String: outH,
        ])
    guard writer.canAdd(vInput) else { throw EngineError(message: "cannot add video input") }
    writer.add(vInput)

    // Camera audio, prototype: one AAC track fed from ExtAudioFile PCM (the
    // /tmp/audio_mix-proven path — video-only AVAssetReader + ExtAudioFile
    // reads, never a reader audio output). Added only when at least one clip
    // carries audio; otherwise the export stays exactly as before.
    let wantAudio = clips.contains {
        if case .video(_, _, _, let h) = $0 { h } else { false }
    }
    var aInput: AVAssetWriterInput? = nil
    var audioFmt: CMAudioFormatDescription? = nil
    if wantAudio {
        let input = AVAssetWriterInput(mediaType: .audio, outputSettings: [
            AVFormatIDKey: kAudioFormatMPEG4AAC as NSNumber,
            AVSampleRateKey: 44100 as NSNumber,
            AVNumberOfChannelsKey: 1 as NSNumber,
            AVEncoderBitRateKey: 64000 as NSNumber])
        input.expectsMediaDataInRealTime = false
        if writer.canAdd(input) {
            var asbd = AudioStreamBasicDescription(mSampleRate: audioRate, mFormatID: kAudioFormatLinearPCM,
                mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked, mBytesPerPacket: 4,
                mFramesPerPacket: 1, mBytesPerFrame: 4, mChannelsPerFrame: 1, mBitsPerChannel: 32, mReserved: 0)
            var fmt: CMAudioFormatDescription?
            if CMAudioFormatDescriptionCreate(allocator: kCFAllocatorDefault, asbd: &asbd,
                layoutSize: 0, layout: nil, magicCookieSize: 0, magicCookie: nil,
                extensions: nil, formatDescriptionOut: &fmt) == noErr, fmt != nil {
                writer.add(input)
                aInput = input
                audioFmt = fmt
            }
        }
    }
    let audioActive = aInput != nil && audioFmt != nil

    func silencePCM(_ seconds: Double) -> Data {
        Data(count: max(0, Int(audioRate * seconds)) * 4)
    }

    // PCM for up to `seconds` of a clip, converted to mono 44.1k float by
    // ExtAudioFile; padded with silence to exactly `seconds` so the audio
    // cursor tracks the video cursor. Unreadable audio degrades to silence —
    // video stays the deliverable, audio is best-effort in this prototype.
    func clipPCM(_ path: String, seconds: Double) -> Data {
        let want = Int(audioRate * seconds)
        guard want > 0 else { return Data() }
        var fref: ExtAudioFileRef?
        guard ExtAudioFileOpenURL(URL(fileURLWithPath: path) as CFURL, &fref) == noErr,
              let f = fref else { return Data(count: want * 4) }
        defer { ExtAudioFileDispose(f) }
        var client = AudioStreamBasicDescription(mSampleRate: audioRate, mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked, mBytesPerPacket: 4,
            mFramesPerPacket: 1, mBytesPerFrame: 4, mChannelsPerFrame: 1, mBitsPerChannel: 32, mReserved: 0)
        guard ExtAudioFileSetProperty(f, kExtAudioFileProperty_ClientDataFormat,
            UInt32(MemoryLayout<AudioStreamBasicDescription>.size), &client) == noErr else {
            return Data(count: want * 4)
        }
        var out = Data()
        out.reserveCapacity(want * 4)
        let chunk: UInt32 = 4096
        let buf = malloc(Int(chunk) * 4)!
        defer { free(buf) }
        var total = 0
        while total < want {
            var abl = AudioBufferList(mNumberBuffers: 1, mBuffers: AudioBuffer(
                mNumberChannels: 1, mDataByteSize: chunk * 4, mData: buf))
            var nframes: UInt32 = min(chunk, UInt32(want - total))
            guard ExtAudioFileRead(f, &nframes, &abl) == noErr, nframes > 0 else { break }
            out.append(contentsOf: UnsafeRawBufferPointer(start: buf, count: Int(nframes) * 4))
            total += Int(nframes)
        }
        if total < want {
            out.append(contentsOf: [UInt8](repeating: 0, count: (want - total) * 4))
        }
        return out
    }

    // Appends mono-44.1k-float PCM at `pts`, chunked so the AAC encoder never
    // sees an unbounded buffer. Stalls throw via waitReady instead of hanging.
    func appendAudioPCM(_ pcm: Data, at pts: CMTime) throws {
        guard let input = aInput, let fd = audioFmt, !pcm.isEmpty else { return }
        var offset = 0
        let totalFrames = pcm.count / 4
        while offset < totalFrames {
            try waitReady(input, what: "highlight audio")
            let chunk = min(4096, totalFrames - offset)
            var block: CMBlockBuffer?
            guard CMBlockBufferCreateWithMemoryBlock(allocator: kCFAllocatorDefault, memoryBlock: nil,
                blockLength: chunk * 4, blockAllocator: kCFAllocatorDefault, customBlockSource: nil,
                offsetToData: 0, dataLength: chunk * 4, flags: 0, blockBufferOut: &block) == kCMBlockBufferNoErr,
                  let bb = block else {
                throw EngineError(message: "highlight audio block failed")
            }
            pcm.withUnsafeBytes { ptr in
                _ = CMBlockBufferReplaceDataBytes(with: ptr.baseAddress! + offset * 4,
                    blockBuffer: bb, offsetIntoDestination: 0, dataLength: chunk * 4)
            }
            var sb: CMSampleBuffer?
            let ts = CMTimeAdd(pts, CMTime(value: Int64(offset), timescale: 44100))
            let status = CMAudioSampleBufferCreateWithPacketDescriptions(allocator: kCFAllocatorDefault,
                dataBuffer: bb, dataReady: true, makeDataReadyCallback: nil, refcon: nil,
                formatDescription: fd, sampleCount: chunk,
                presentationTimeStamp: ts, packetDescriptions: nil, sampleBufferOut: &sb)
            guard status == noErr, let audioSB = sb, input.append(audioSB) else {
                throw EngineError(message: "highlight audio append failed: \(status)")
            }
            offset += chunk
        }
    }

    // Video-only by design (for now): any audio-track interaction — even a
    // bare tracks(.audio) query with no reader — poisons later video decoding
    // of the same file in-process on macOS 26 (observed across 5 probes: 0
    // audio calls → 90/90 frames; ≥1 audio call → stall at 42–43 frames).
    // Camera-audio mixing is deferred until audio can be extracted off the
    // AVAssetReader path (e.g. ExtAudioFile) or on the main thread.

    guard writer.startWriting() else {
        throw EngineError(message: "writer start failed: \(writer.error.map(String.init(describing:)) ?? "unknown")")
    }
    writer.startSession(atSourceTime: .zero)

    // A failed writer never flips isReadyForMoreMediaData, so a bare sleep
    // loop would spin forever (the observed >60s hang on real MP4s). Cap the
    // wait and surface the writer error instead. `what` locates the stall.
    func waitReady(_ input: AVAssetWriterInput, what: String) throws {
        var spins = 0
        while !input.isReadyForMoreMediaData {
            if writer.status == .failed || writer.status == .cancelled {
                throw EngineError(message: "highlight writer failed at \(what): \(writer.error.map(String.init(describing:)) ?? "unknown")")
            }
            Thread.sleep(forTimeInterval: 0.01)
            spins += 1
            if spins > 3000 {
                throw EngineError(message: "highlight writer stalled at \(what) (30s without accepting data, status \(writer.status.rawValue))")
            }
        }
    }

    func makePixelBuffer(_ image: CGImage, index: Int64, zoom: CGFloat = 1, driftY: CGFloat = 0) throws -> CVPixelBuffer {
        var pixelBuffer: CVPixelBuffer?
        let status = CVPixelBufferPoolCreatePixelBuffer(
            nil, adaptor.pixelBufferPool!, &pixelBuffer)
        guard status == kCVReturnSuccess, let pb = pixelBuffer else {
            throw EngineError(message: "pixel buffer pool unavailable")
        }
        CVPixelBufferLockBaseAddress(pb, [])
        defer { CVPixelBufferUnlockBaseAddress(pb, []) }
        guard let base = CVPixelBufferGetBaseAddress(pb) else {
            throw EngineError(message: "pixel buffer base unavailable")
        }
        let bytesPerRow = CVPixelBufferGetBytesPerRow(pb)
        let colorSpace = CGColorSpaceCreateDeviceRGB()
        let ctx = CGContext(
            data: base, width: outW, height: outH, bitsPerComponent: 8,
            bytesPerRow: bytesPerRow, space: colorSpace,
            bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue
                | CGBitmapInfo.byteOrder32Little.rawValue)
        guard let ctx else { throw EngineError(message: "cannot create buffer context") }
        ctx.setFillColor(CGColor(gray: 0.05, alpha: 1.0))
        ctx.fill(CGRect(x: 0, y: 0, width: outW, height: outH))
        // Aspect-fill, plus the Ken Burns drift: callers ease zoom 1.00→1.10
        // with a slight upward pan across a still's hold. driftY is a
        // fraction of the canvas height.
        let scale = max(CGFloat(outW) / CGFloat(image.width), CGFloat(outH) / CGFloat(image.height)) * zoom
        let drawW = CGFloat(image.width) * scale
        let drawH = CGFloat(image.height) * scale
        ctx.draw(image, in: CGRect(
            x: (CGFloat(outW) - drawW) / 2,
            y: (CGFloat(outH) - drawH) / 2 + driftY * CGFloat(outH),
            width: drawW, height: drawH))
        _ = index
        return pb
    }

    var frame: Int64 = 0
    var photos = 0
    var cards = 0
    var videosIncluded = 0
    var videoFrames = 0
    var audioClips = 0
    var audioFrames: Int64 = 0
    var clipErrors: [String] = []
    var cursor = CMTime.zero
    var audioCursor = CMTime.zero
    let ciCtx = CIContext()

    // Center-cropped CGImage covering the 16:9 canvas from a decoded frame.
    // AVAssetReader yields stored pixels; the track's preferredTransform
    // (e.g. the 180° rotation vivo footage carries) must be applied first,
    // or clips bake in upside-down/sideways. The crop stays centered in the
    // transformed extent (origin-aware: 180° puts it at negative offsets).
    func canvasCrop(_ sample: CVPixelBuffer, transform: CGAffineTransform) -> CGImage? {
        var ci = CIImage(cvPixelBuffer: sample)
        if !transform.isIdentity {
            ci = ci.transformed(by: transform)
        }
        let ex = ci.extent
        let w = ex.width, h = ex.height
        guard w > 0, h > 0 else { return nil }
        let scale = max(CGFloat(outW) / w, CGFloat(outH) / h)
        let crop = CGRect(
            x: ex.origin.x + (w - CGFloat(outW) / scale) / 2,
            y: ex.origin.y + (h - CGFloat(outH) / scale) / 2,
            width: CGFloat(outW) / scale, height: CGFloat(outH) / scale)
        return ciCtx.createCGImage(ci, from: crop)
    }

    // Streams one video clip (up to `take` seconds) onto the timeline:
    // decoded frames drawn on the canvas. Returns false when the clip
    // contributed no frames.
    func appendClip(asset: AVURLAsset, vTrack: AVAssetTrack, take: Double) throws -> Bool {
        // The asset and video track arrive from classification: no tracks()
        // calls in here (see the Clip comment — re-querying races on macOS
        // 26, and any audio-track interaction poisons video decoding).
        // Exactly one reader per clip, video output only.
        let reader: AVAssetReader
        do {
            reader = try AVAssetReader(asset: asset)
        } catch {
            return false
        }
        let vOut = AVAssetReaderTrackOutput(track: vTrack, outputSettings: [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        ])
        reader.add(vOut)
        guard reader.startReading() else {
            return false
        }

        var wroteAny = false
            while let sb = vOut.copyNextSampleBuffer() {
                let pts = CMSampleBufferGetPresentationTimeStamp(sb)
                if pts.seconds >= take { break }
                guard let pb = CMSampleBufferGetImageBuffer(sb),
                      let cg = canvasCrop(pb, transform: vTrack.preferredTransform) else { continue }
                // Native sample timing (no re-quantization): dense source
                // framerates would otherwise collapse onto identical stamps
                // and every append would fail.
                let t = CMTimeAdd(cursor, pts)
            try waitReady(vInput, what: "clip video frame")
                let out = try makePixelBuffer(cg, index: frame)
                adaptor.append(out, withPresentationTime: t)
                frame += 1
                videoFrames += 1
                wroteAny = true
        }
        reader.cancelReading()
        return wroteAny
    }

        // Per-clip progress for the host (same "progress" channel as scan):
    // photos land ~1/sec; a heavy video clip holds `done` still, then it
    // jumps — the count is honest about finished units, not wall time.
    let totalClips = clips.count
    var doneClips = 0
    for clip in clips {
        if cursor.seconds >= cap { break }
        switch clip {
        case .photo(let image):
            let budget = Int((cap - cursor.seconds) * Double(fps))
            let total = min(framesPerPhoto, budget)
            let spanStart = cursor
            for j in 0..<total {
                try waitReady(vInput, what: "photo frame")
                // Ken Burns: ease 1.00→1.10 zoom with a gentle upward drift
                // across the hold so stills feel like slow camera moves.
                let t = total > 1 ? Double(j) / Double(total - 1) : 0
                let pb = try makePixelBuffer(image, index: frame, zoom: 1.0 + 0.10 * t, driftY: 0.02 * t)
                adaptor.append(pb, withPresentationTime: cursor)
                frame += 1
                cursor = CMTimeAdd(cursor, CMTime(value: 1, timescale: CMTimeScale(fps)))
            }
            photos += 1
            if audioActive {
                // Stills are silent by design; explicit silence keeps the
                // audio cursor glued to the video cursor.
                let span = (cursor - spanStart).seconds
                try appendAudioPCM(silencePCM(span), at: audioCursor)
                audioFrames += Int64(audioRate * span)
                audioCursor = CMTimeAdd(audioCursor, CMTime(seconds: span, preferredTimescale: 44100))
            }
        case .titlecard(let image):
            let budget = Int((cap - cursor.seconds) * Double(fps))
            let spanStart = cursor
            for _ in 0..<min(framesPerPhoto, budget) {
                try waitReady(vInput, what: "title card")
                let pb = try makePixelBuffer(image, index: frame)
                adaptor.append(pb, withPresentationTime: cursor)
                frame += 1
                cursor = CMTimeAdd(cursor, CMTime(value: 1, timescale: CMTimeScale(fps)))
            }
            cards += 1
            if audioActive {
                let span = (cursor - spanStart).seconds
                try appendAudioPCM(silencePCM(span), at: audioCursor)
                audioFrames += Int64(audioRate * span)
                audioCursor = CMTimeAdd(audioCursor, CMTime(seconds: span, preferredTimescale: 44100))
            }
        case .video(let path, let asset, let vTrack, let hasAudio):
            let take = min(secondsPerClip, cap - cursor.seconds)
            // Audio must never outrun video: a stalled/skipped clip gets
            // silence for its span (the waitReady cap turns the pool stall
            // into a loud skip instead of a hang).
            var wroteVideo = false
            do {
                if try appendClip(asset: asset, vTrack: vTrack, take: take) {
                    videosIncluded += 1
                    wroteVideo = true
                } else {
                    skipped += 1
                }
            } catch {
                skipped += 1
                // Surfaced (not just counted) so callers can tell a corrupt
                // file from a pipeline stall without guessing.
                clipErrors.append("\((path as NSString).lastPathComponent): \(error)")
            }
            if audioActive {
                let pcm: Data
                if hasAudio && wroteVideo {
                    pcm = clipPCM(path, seconds: take)
                    audioClips += 1
                } else {
                    pcm = silencePCM(take)
                }
                try appendAudioPCM(pcm, at: audioCursor)
                audioFrames += Int64(pcm.count / 4)
                audioCursor = CMTimeAdd(audioCursor, CMTime(seconds: take, preferredTimescale: 44100))
            }
            cursor = CMTimeAdd(cursor, CMTime(seconds: take, preferredTimescale: CMTimeScale(fps)))
        }
        doneClips += 1
        write(makeEvent(id: requestID, event: "progress", data: [
            "done": doneClips, "total": totalClips,
        ]))
    }
    vInput.markAsFinished()
    aInput?.markAsFinished()
    let sem = DispatchSemaphore(value: 0)
    writer.finishWriting { sem.signal() }
    sem.wait()
    guard writer.status == .completed else {
        throw EngineError(message: "highlight export failed: \(writer.error.map(String.init(describing:)) ?? "unknown")")
    }
    let attrs = try FileManager.default.attributesOfItem(atPath: output)
    return [
        "output": output,
        "bytes": (attrs[.size] as? NSNumber)?.int64Value ?? 0,
        "seconds": cursor.seconds,
        "photos": photos,
        "cards": cards,
        "videosIncluded": videosIncluded,
        "videosSkipped": skipped,
        "videoFrames": videoFrames,
        "audioClips": audioClips,
        "audioFrames": audioFrames,
        "clipErrors": clipErrors,
    ]
}


// MARK: - Distribution compression

/// Recodes one image to a bounded JPEG (WhatsApp-friendly: 1280px, q0.72).
func compressImage(path: String, output: String, maxPx: Int, quality: Double) throws -> [String: Any] {
    let src = CGImageSourceCreateWithURL(
        URL(fileURLWithPath: path) as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary)
    guard let src else { throw EngineError(message: "not an image: \(path)") }
    let opts: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceThumbnailMaxPixelSize: max(64, min(maxPx, 4096)),
    ]
    guard let cg = CGImageSourceCreateThumbnailAtIndex(src, 0, opts as CFDictionary) else {
        throw EngineError(message: "cannot decode image: \(path)")
    }
    let outURL = URL(fileURLWithPath: output) as CFURL
    guard
        let dest = CGImageDestinationCreateWithURL(outURL, "public.jpeg" as CFString, 1, nil)
    else { throw EngineError(message: "cannot write: \(output)") }
    CGImageDestinationAddImage(
        dest, cg, [kCGImageDestinationLossyCompressionQuality: min(max(quality, 0.1), 1.0)] as CFDictionary)
    guard CGImageDestinationFinalize(dest) else {
        throw EngineError(message: "cannot finalize: \(output)")
    }
    let attrs = try FileManager.default.attributesOfItem(atPath: output)
    return [
        "output": output,
        "bytes": (attrs[.size] as? NSNumber)?.int64Value ?? 0,
    ]
}

/// Recodes one video to a bounded MP4 (WhatsApp-friendly: 1280x720 H.264,
/// fast-start for network sharing). Throws when the source has no video
/// track or the export session cannot complete.
func compressVideo(path: String, output: String) throws -> [String: Any] {
    let asset = AVURLAsset(url: URL(fileURLWithPath: path))
    guard asset.tracks(withMediaType: .video).first != nil else {
        throw EngineError(message: "no video track: \(path)")
    }
    guard let session = AVAssetExportSession(asset: asset, presetName: AVAssetExportPreset1280x720) else {
        throw EngineError(message: "no export session for: \(path)")
    }
    try? FileManager.default.removeItem(atPath: output)
    session.outputURL = URL(fileURLWithPath: output)
    session.outputFileType = .mp4
    session.shouldOptimizeForNetworkUse = true
    let sem = DispatchSemaphore(value: 0)
    session.exportAsynchronously { sem.signal() }
    sem.wait()
    guard session.status == .completed else {
        throw EngineError(message: "video export failed: \(session.error.map(String.init(describing:)) ?? "unknown")")
    }
    let attrs = try FileManager.default.attributesOfItem(atPath: output)
    return [
        "output": output,
        "bytes": (attrs[.size] as? NSNumber)?.int64Value ?? 0,
    ]
}

/// Privacy copy: full-resolution bytes with location/metadata stripped.
/// Images decode fully and re-encode as JPEG (q0.92) with a fresh, empty
/// metadata dictionary — no EXIF, no GPS. Videos remux with the passthrough
/// preset and an empty metadata filter, so the media is bit-identical while
/// container-level location and creation tags are dropped. The compact
/// recode path needs no such step (thumbnail-API output carries no metadata
/// at all — probed clean); this exists for full-quality sharing.
func stripCopy(path: String, output: String) throws -> [String: Any] {
    let ext = (path as NSString).pathExtension.lowercased()
    try? FileManager.default.removeItem(atPath: output)
    if VIDEO_EXTS.contains(ext) {
        let asset = AVURLAsset(url: URL(fileURLWithPath: path))
        guard asset.tracks(withMediaType: .video).first != nil else {
            throw EngineError(message: "no video track: \(path)")
        }
        guard let session = AVAssetExportSession(asset: asset, presetName: AVAssetExportPresetPassthrough) else {
            throw EngineError(message: "no export session for: \(path)")
        }
        session.outputURL = URL(fileURLWithPath: output)
        session.outputFileType = .mp4
        session.metadataItemFilter = .forSharing()
        let sem = DispatchSemaphore(value: 0)
        session.exportAsynchronously { sem.signal() }
        sem.wait()
        guard session.status == .completed else {
            throw EngineError(message: "strip export failed: \(session.error.map(String.init(describing:)) ?? "unknown")")
        }
    } else {
        guard let src = CGImageSourceCreateWithURL(
            URL(fileURLWithPath: path) as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
              let cg = CGImageSourceCreateImageAtIndex(src, 0, nil)
        else { throw EngineError(message: "cannot decode image: \(path)") }
        let outURL = URL(fileURLWithPath: output) as CFURL
        guard
            let dest = CGImageDestinationCreateWithURL(outURL, "public.jpeg" as CFString, 1, nil)
        else { throw EngineError(message: "cannot write: \(output)") }
        // No properties dictionary: nothing carries over, GPS included.
        CGImageDestinationAddImage(
            dest, cg, [kCGImageDestinationLossyCompressionQuality: 0.92] as CFDictionary)
        guard CGImageDestinationFinalize(dest) else {
            throw EngineError(message: "cannot finalize: \(output)")
        }
    }
    let attrs = try FileManager.default.attributesOfItem(atPath: output)
    return [
        "output": output,
        "bytes": (attrs[.size] as? NSNumber)?.int64Value ?? 0,
    ]
}

// MARK: - Batch hashing

/// Hashes many files in one call so the Rust side pays a single round-trip
/// per batch instead of one per asset. Individual failures are reported per
/// item so one unreadable file cannot abort a batch.
func hashBatch(paths: [String]) -> [String: Any] {
    var results: [[String: Any]] = []
    results.reserveCapacity(paths.count)
    for path in paths {
        do {
            var entry = try hashAsset(path: path)
            entry["ok"] = true
            results.append(entry)
        } catch {
            results.append(["path": path, "ok": false, "error": "\(error)"])
        }
    }
    return ["count": paths.count, "results": results]
}

/// 64-bit difference hash: 9x8 grayscale thumbnail, one bit per adjacent
/// column pair. Invariant to resize/re-encode; Hamming distance <= 8 reads
/// as "visually the same shot". Pure ImageIO, no model, milliseconds.
func dHash(path: String) throws -> String {
    guard let src = CGImageSourceCreateWithURL(
        URL(fileURLWithPath: path) as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary)
    else { throw EngineError(message: "not an image: \(path)") }
    let thumbOpts: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceThumbnailMaxPixelSize: 64,
    ]
    guard let thumb = CGImageSourceCreateThumbnailAtIndex(src, 0, thumbOpts as CFDictionary) else {
        throw EngineError(message: "cannot decode image: \(path)")
    }
    let w = 9, h = 8
    var pixels = [UInt8](repeating: 0, count: w * h)
    let colorSpace = CGColorSpaceCreateDeviceGray()
    guard let ctx = CGContext(
        data: &pixels, width: w, height: h, bitsPerComponent: 8,
        bytesPerRow: w, space: colorSpace,
        bitmapInfo: CGImageAlphaInfo.none.rawValue)
    else { throw EngineError(message: "cannot rasterize: \(path)") }
    ctx.interpolationQuality = .high
    ctx.draw(thumb, in: CGRect(x: 0, y: 0, width: w, height: h))
    var hash: UInt64 = 0
    for y in 0..<h {
        for x in 0..<(w - 1) {
            hash <<= 1
            if pixels[y * w + x] > pixels[y * w + x + 1] { hash |= 1 }
        }
    }
    return String(format: "%016llx", hash)
}

func dHashBatch(paths: [String]) -> [String: Any] {
    var results: [[String: Any]] = []
    results.reserveCapacity(paths.count)
    for path in paths {
        do {
            results.append(["path": path, "ok": true, "dhash": try dHash(path: path)])
        } catch {
            results.append(["path": path, "ok": false, "error": "\(error)"])
        }
    }
    return ["count": paths.count, "results": results]
}

// MARK: - Visual similarity embeddings

/// Image-query similarity encoder: Apple's built-in Vision feature print
/// (VNGenerateImageFeaturePrint, 2048 f32, zero download — the v1 pick over
/// MobileCLIP per docs/plans/similar-photos.md). Returned as base64 of the
/// raw float bytes plus the element count, so the Rust side can
/// cosine-compare without re-decoding images.
func embedImage(path: String) throws -> [String: Any] {
    guard
        let src = CGImageSourceCreateWithURL(
            URL(fileURLWithPath: path) as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
        let cg = CGImageSourceCreateImageAtIndex(src, 0, nil)
    else { throw EngineError(message: "cannot decode image: \(path)") }
    let req = VNGenerateImageFeaturePrintRequest()
    do {
        try VNImageRequestHandler(cgImage: cg, options: [:]).perform([req])
    } catch {
        throw EngineError(message: "feature print failed: \(error)")
    }
    guard let obs = req.results?.first as? VNFeaturePrintObservation else {
        throw EngineError(message: "no feature print for: \(path)")
    }
    let raw = obs.data
    let dim = obs.elementCount
    if obs.elementType == .double {
        // Normalize doubles to f32 so the wire format is always f32 LE.
        let count = raw.count / 8
        var floats = [Float](repeating: 0, count: count)
        raw.withUnsafeBytes { ptr in
            let doubles = ptr.bindMemory(to: Double.self)
            for i in 0..<count { floats[i] = Float(doubles[i]) }
        }
        let bytes = floats.withUnsafeBytes { Data($0) }
        return ["path": path, "dim": count, "vector": bytes.base64EncodedString()]
    }
    guard obs.elementType == .float, raw.count == dim * 4 else {
        throw EngineError(message: "unsupported feature-print element type for: \(path)")
    }
    return ["path": path, "dim": dim, "vector": raw.base64EncodedString()]
}

/// Batch encoder mirroring dHashBatch, with scan-style progress events so
/// the Rust side can stream `embed-progress` per flush.
func embedBatch(paths: [String]) -> [String: Any] {
    var results: [[String: Any]] = []
    results.reserveCapacity(paths.count)
    for (i, path) in paths.enumerated() {
        do {
            var entry = try embedImage(path: path)
            entry["ok"] = true
            results.append(entry)
        } catch {
            results.append(["path": path, "ok": false, "error": "\(error)"])
        }
        if (i + 1) % 20 == 0 || i + 1 == paths.count {
            write(makeEvent(id: requestID, event: "progress", data: [
                "processed": i + 1, "total": paths.count,
            ]))
        }
    }
    return ["count": paths.count, "results": results]
}

// MARK: - CLIP semantic search (OpenCLIP ViT-B-32, CoreML fp16)
//
// Text + image encoders for semantic search. Models download once (MIT
// weights) into the app models dir; the BPE vocab ships in-repo and its path
// arrives per command (the sidecar has no bundle resources). Wire format
// matches the face descriptors (base64 f32 LE + dim) so Rust stores and
// cosine-compares without re-decoding. Embeddings are L2-normalized here.

// Loaded once per process; the sidecar is long-lived.
var clipImageModel: MLModel? = nil
var clipTextModel: MLModel? = nil
var clipModelDir: String? = nil

func clipModels(modelDir: String) throws -> (MLModel, MLModel) {
    if let i = clipImageModel, let t = clipTextModel, clipModelDir == modelDir {
        return (i, t)
    }
    // mlpackage → compiled mlmodelc on first use (writable models dir).
    func load(_ name: String) throws -> MLModel {
        let dst = URL(fileURLWithPath: modelDir + "/\(name).mlmodelc")
        if !FileManager.default.fileExists(atPath: dst.path) {
            let src = URL(fileURLWithPath: modelDir + "/\(name).mlpackage")
            let compiled = try MLModel.compileModel(at: src)
            if FileManager.default.fileExists(atPath: dst.path) {
                try FileManager.default.removeItem(at: dst)
            }
            try FileManager.default.moveItem(at: compiled, to: dst)
        }
        return try MLModel(contentsOf: dst)
    }
    let i = try load("CLIPImage")
    let t = try load("CLIPText")
    clipImageModel = i
    clipTextModel = t
    clipModelDir = modelDir
    return (i, t)
}

/// Minimal gzip member reader (header + raw deflate + ignored trailer) for
/// the shipped BPE vocab. Compression.framework has no gunzip one-shot.
func gunzip(_ data: Data) throws -> Data {
    guard data.count > 18, data[0] == 0x1f, data[1] == 0x8b, data[2] == 0x08 else {
        throw EngineError(message: "not a gzip member")
    }
    let flags = data[3]
    var off = 10
    func u16(_ o: Int) -> Int { Int(data[o]) | (Int(data[o + 1]) << 8) }
    if flags & 0x04 != 0 {
        let xlen = u16(off)
        off += 2 + xlen
    }
    if flags & 0x08 != 0 {
        while off < data.count && data[off] != 0 { off += 1 }
        off += 1
    }
    if flags & 0x10 != 0 {
        while off < data.count && data[off] != 0 { off += 1 }
        off += 1
    }
    if flags & 0x02 != 0 { off += 2 }
    let end = data.count - 8
    guard end > off else { throw EngineError(message: "truncated gzip member") }
    // One-shot zlib decode into a bounded buffer (the vocab inflates to
    // ~3.2MB; 8MB headroom with a strict fullness check).
    let maxOut = 8 << 20
    var out = Data(count: maxOut)
    let decoded = out.withUnsafeMutableBytes { dst -> Int in
        data.withUnsafeBytes { src -> Int in
            compression_decode_buffer(dst.baseAddress!, maxOut,
                src.baseAddress! + off, end - off, nil, COMPRESSION_ZLIB)
        }
    }
    guard decoded > 0 && decoded < maxOut else {
        throw EngineError(message: "zlib decode failed")
    }
    out.count = decoded
    return out
}

/// Byte-level BPE ported from open_clip's SimpleTokenizer (MIT, OpenAI CLIP
/// lineage): same merges file, same regex, same SOT/EOT/pad/truncate. ASCII
/// parity is verified against the Python tokenizer; the port skips ftfy/html
/// unescaping (mojibake repair), which search queries don't need.
struct ClipBPE {
    let encoder: [String: Int]
    let ranks: [String: Int]
    let byteEncoder: [UInt8: String]
    let pattern: NSRegularExpression
    var cache: [String: [String]] = [:]
    let sot = 49406
    let eot = 49407

    static func byteMap() -> [UInt8: String] {
        var bs: [Int] = Array(33...126) + Array(161...172) + Array(174...255)
        var cs = bs
        var n = 0
        for b in 0..<256 where !bs.contains(b) {
            bs.append(b)
            cs.append(256 + n)
            n += 1
        }
        var m: [UInt8: String] = [:]
        for (b, c) in zip(bs, cs) {
            m[UInt8(b)] = String(UnicodeScalar(c)!)
        }
        return m
    }

    static func load(vocabPath: String) throws -> ClipBPE {
        let raw = try Data(contentsOf: URL(fileURLWithPath: vocabPath))
        let text = try String(data: gunzip(raw), encoding: .utf8)
            .map { $0.components(separatedBy: "\n") } ?? []
        // Mirrors open_clip's slice: header line + 48895 merges.
        let lines = Array(text.dropFirst().prefix(48895))
        var merges: [(String, String)] = []
        merges.reserveCapacity(lines.count)
        for line in lines {
            let parts = line.split(separator: " ", maxSplits: 1).map(String.init)
            guard parts.count == 2 else { continue }
            merges.append((parts[0], parts[1]))
        }
        let bm = byteMap()
        // Byte order must mirror bytes_to_unicode() insertion order (dict
        // .values is unordered): printable ASCII, high bytes, then the rest.
        var byteOrder: [Int] = Array(33...126) + Array(161...172) + Array(174...255)
        var seen = Set(byteOrder)
        for b in 0..<256 where !seen.contains(b) {
            byteOrder.append(b)
            seen.insert(b)
        }
        let bytes = byteOrder.map { bm[UInt8($0)]! }
        var vocab = bytes + bytes.map { $0 + "</w>" }
        vocab.reserveCapacity(49408)
        for (a, b) in merges { vocab.append(a + b) }
        vocab.append("<start_of_text>")
        vocab.append("<end_of_text>")
        var encoder: [String: Int] = [:]
        encoder.reserveCapacity(vocab.count)
        for (i, v) in vocab.enumerated() { encoder[v] = i }
        var ranks: [String: Int] = [:]
        ranks.reserveCapacity(merges.count)
        for (i, m) in merges.enumerated() { ranks[m.0 + "\u{1F}" + m.1] = i }
        let pattern = try NSRegularExpression(
            pattern: "<start_of_text>|<end_of_text>|'s|'t|'re|'ve|'m|'ll|'d|[\\p{L}]+|[\\p{N}]|[^\\s\\p{L}\\p{N}]+",
            options: [.caseInsensitive])
        return ClipBPE(encoder: encoder, ranks: ranks, byteEncoder: bm, pattern: pattern)
    }

    mutating func bpe(_ token: String) -> [String] {
        if let hit = cache[token] { return hit }
        var word: [String] = token.dropLast().map { String($0) }
        guard let last = token.last else {
            cache[token] = ["</w>"]
            return ["</w>"]
        }
        word.append(String(last) + "</w>")
        while word.count > 1 {
            var bestRank = Int.max
            var best = ""
            for i in 0..<(word.count - 1) {
                let key = word[i] + "\u{1F}" + word[i + 1]
                let r = ranks[key] ?? Int.max
                if r < bestRank {
                    bestRank = r
                    best = key
                }
            }
            if bestRank == Int.max { break }
            let parts = best.components(separatedBy: "\u{1F}")
            let first = parts[0]
            let second = parts[1]
            var merged: [String] = []
            var i = 0
            while i < word.count {
                guard let j = word[i...].firstIndex(of: first) else {
                    merged.append(contentsOf: word[i...])
                    break
                }
                merged.append(contentsOf: word[i..<j])
                i = j
                if word[i] == first && i < word.count - 1 && word[i + 1] == second {
                    merged.append(first + second)
                    i += 2
                } else {
                    merged.append(word[i])
                    i += 1
                }
            }
            word = merged
            if word.count == 1 { break }
        }
        cache[token] = word
        return word
    }

    mutating func encode(_ text: String) -> [Int] {
        // open_clip clean='lower': whitespace collapse + lowercase.
        let cleaned = text.components(separatedBy: .whitespacesAndNewlines)
            .filter { !$0.isEmpty }.joined(separator: " ").lowercased()
        var ids: [Int] = []
        let ns = cleaned as NSString
        let matches = pattern.matches(in: cleaned, range: NSRange(location: 0, length: ns.length))
        for m in matches {
            let tok = ns.substring(with: m.range)
            let mapped = tok.utf8.map { byteEncoder[$0]! }.joined()
            for piece in bpe(mapped) {
                if let id = encoder[piece] { ids.append(id) }
            }
        }
        return ids
    }

    mutating func tokenize(_ text: String) -> [Int32] {
        var ids = [sot] + encode(text) + [eot]
        if ids.count > 77 {
            ids = Array(ids.prefix(77))
            ids[76] = eot
        }
        var out = [Int32](repeating: 0, count: 77)
        for (i, v) in ids.enumerated() { out[i] = Int32(v) }
        return out
    }
}

/// Vocab ships next to the sidecar binary (copied there by build-engine.sh):
/// no bundle resources, no per-call path plumbing — dev and prod identical.
func clipVocabURL() throws -> URL {
    let exe = URL(fileURLWithPath: CommandLine.arguments[0]).deletingLastPathComponent()
    let url = exe.appendingPathComponent("clip_bpe_vocab.txt.gz")
    guard FileManager.default.isReadableFile(atPath: url.path) else {
        throw EngineError(message: "CLIP vocab missing next to engine binary: \(url.path)")
    }
    return url
}

var clipBPELock = NSLock()
var clipBPEInst: ClipBPE? = nil

/// Process-wide BPE tables (vocab parse is ~0.3s; backfills must not redo it).
func sharedBPE() throws -> ClipBPE {
    clipBPELock.lock()
    defer { clipBPELock.unlock() }
    if let inst = clipBPEInst { return inst }
    let inst = try ClipBPE.load(vocabPath: clipVocabURL().path)
    clipBPEInst = inst
    return inst
}

/// CLIP preprocess: resize short edge to 224 (bicubic), center-crop 224,
/// normalize with the OpenAI mean/std. Output is CHW float32.
func clipPreprocess(_ cg: CGImage) throws -> MLMultiArray {
    let w = cg.width
    let h = cg.height
    guard w > 0, h > 0 else { throw EngineError(message: "empty image for CLIP") }
    let scale = 224.0 / Double(min(w, h))
    let rw = max(224, Int((Double(w) * scale).rounded()))
    let rh = max(224, Int((Double(h) * scale).rounded()))
    let space = CGColorSpaceCreateDeviceRGB()
    guard let rctx = CGContext(data: nil, width: rw, height: rh, bitsPerComponent: 8,
        bytesPerRow: 0, space: space, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else {
        throw EngineError(message: "CLIP resize context failed")
    }
    rctx.interpolationQuality = .high
    rctx.draw(cg, in: CGRect(x: 0, y: 0, width: rw, height: rh))
    guard let resized = rctx.makeImage(),
          let crop = resized.cropping(to: CGRect(x: (rw - 224) / 2, y: (rh - 224) / 2, width: 224, height: 224)) else {
        throw EngineError(message: "CLIP crop failed")
    }
    var px = [UInt8](repeating: 0, count: 224 * 224 * 4)
    guard let cctx = CGContext(data: &px, width: 224, height: 224, bitsPerComponent: 8,
        bytesPerRow: 224 * 4, space: space,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else {
        throw EngineError(message: "CLIP pixel context failed")
    }
    cctx.draw(crop, in: CGRect(x: 0, y: 0, width: 224, height: 224))
    let mean: [Float] = [0.48145466, 0.4578275, 0.40821073]
    let std: [Float] = [0.26862954, 0.26130258, 0.27577711]
    let ma = try MLMultiArray(shape: [1, 3, 224, 224] as [NSNumber], dataType: .float32)
    for y in 0..<224 {
        for x in 0..<224 {
            let o = (y * 224 + x) * 4
            let r = Float(px[o]) / 255.0
            let g = Float(px[o + 1]) / 255.0
            let b = Float(px[o + 2]) / 255.0
            ma[[0, 0, y, x] as [NSNumber]] = NSNumber(value: (r - mean[0]) / std[0])
            ma[[0, 1, y, x] as [NSNumber]] = NSNumber(value: (g - mean[1]) / std[1])
            ma[[0, 2, y, x] as [NSNumber]] = NSNumber(value: (b - mean[2]) / std[2])
        }
    }
    return ma
}

/// Runs one CLIP tower and returns the L2-normalized f32 vector as base64
/// (same wire format as the face descriptors) plus its dimension.
func clipPredict(_ model: MLModel, input: MLMultiArray, name: String) throws -> (String, Int) {
    let out = try model.prediction(from: MLDictionaryFeatureProvider(dictionary: [name: input]))
    guard let outName = out.featureNames.first,
          let mv = out.featureValue(for: outName)?.multiArrayValue else {
        throw EngineError(message: "CLIP model returned no vector")
    }
    let n = mv.count
    var v = [Float](repeating: 0, count: n)
    if mv.dataType == .float16 {
        let p = mv.dataPointer.bindMemory(to: UInt16.self, capacity: n)
        for i in 0..<n { v[i] = Float(Float16(bitPattern: p[i])) }
    } else if mv.dataType == .float32 {
        let p = mv.dataPointer.bindMemory(to: Float.self, capacity: n)
        for i in 0..<n { v[i] = p[i] }
    } else {
        throw EngineError(message: "CLIP model returned dtype \(mv.dataType.rawValue)")
    }
    var norm: Double = 0
    for x in v { norm += Double(x) * Double(x) }
    norm = norm.squareRoot()
    guard norm > 0 else { throw EngineError(message: "CLIP model returned zero vector") }
    let nv = v.map { $0 / Float(norm) }
    return (nv.withUnsafeBytes { Data($0) }.base64EncodedString(), n)
}

func clipEncodeText(_ text: String, modelDir: String) throws -> [String: Any] {
    let (_, tm) = try clipModels(modelDir: modelDir)
    var bpe = try sharedBPE()
    let ids = bpe.tokenize(text)
    let ma = try MLMultiArray(shape: [1, 77] as [NSNumber], dataType: .int32)
    for (i, t) in ids.enumerated() { ma[i] = NSNumber(value: t) }
    let (vec, dim) = try clipPredict(tm, input: ma, name: "tokens")
    return ["vector": vec, "dim": dim]
}

func clipEncodeImage(path: String, modelDir: String) throws -> [String: Any] {
    let (im, _) = try clipModels(modelDir: modelDir)
    guard let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL,
        [kCGImageSourceShouldCache: false] as CFDictionary),
          let cg = CGImageSourceCreateImageAtIndex(src, 0, nil) else {
        throw EngineError(message: "cannot decode image: \(path)")
    }
    let (vec, dim) = try clipPredict(im, input: clipPreprocess(cg), name: "image")
    return ["path": path, "vector": vec, "dim": dim]
}

/// Evenly spaced keyframe embeddings for video search: decode at `every`
/// seconds (up to `maxFrames`), embed each with the image tower. The step is
/// widened when the clip is longer than `every * maxFrames`, so the sample
/// covers the **whole** duration instead of only the opening seconds (the
/// first cut indexed 24s of every clip and made the rest unfindable).
/// Tolerated (non-exact) seeks keep this fast; timestamps come from the
/// actual frame.
func clipKeyframes(path: String, modelDir: String, every: Double, maxFrames: Int) throws -> [String: Any] {
    let asset = AVURLAsset(url: URL(fileURLWithPath: path))
    let dur = asset.duration.seconds
    guard dur.isFinite && dur > 0 else { throw EngineError(message: "no video duration: \(path)") }
    let (im, _) = try clipModels(modelDir: modelDir)
    let gen = AVAssetImageGenerator(asset: asset)
    gen.appliesPreferredTrackTransform = true
    gen.requestedTimeToleranceBefore = CMTime(seconds: 0.5, preferredTimescale: 600)
    gen.requestedTimeToleranceAfter = CMTime(seconds: 0.5, preferredTimescale: 600)
    let target = max(1, maxFrames)
    let step = max(0.5, max(every, dur / Double(target)))
    var times: [CMTime] = []
    var t = 0.0
    while t < dur && times.count < target {
        times.append(CMTime(seconds: t, preferredTimescale: 600))
        t += step
    }
    var frames: [[String: Any]] = []
    for (i, time) in times.enumerated() {
        var actual = CMTime.zero
        guard let cg = try? gen.copyCGImage(at: time, actualTime: &actual) else { continue }
        do {
            let (vec, dim) = try clipPredict(im, input: clipPreprocess(cg), name: "image")
            frames.append(["t": actual.seconds, "vector": vec, "dim": dim])
        } catch {
            continue
        }
        if (i + 1) % 4 == 0 || i + 1 == times.count {
            write(makeEvent(id: requestID, event: "progress", data: [
                "processed": i + 1, "total": times.count,
            ]))
        }
    }
    return ["path": path, "frames": frames]
}

// MARK: - Image quality
/// Variance of the Laplacian over a downscaled luma image: the classic focus /
/// sharpness measure. Cheap, camera-agnostic and good enough to separate
/// motion-blurred shots from sharp ones.
func sharpnessScore(path: String, maxPx: Int) throws -> Double {
    let px = max(32, min(maxPx, 2048))
    let srcOptions: [CFString: Any] = [kCGImageSourceShouldCache: false]
    guard
        let src = CGImageSourceCreateWithURL(
            URL(fileURLWithPath: path) as CFURL, srcOptions as CFDictionary)
    else { throw EngineError(message: "not an image: \(path)") }

    let thumbOptions: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceThumbnailMaxPixelSize: px,
    ]
    guard
        let cg = CGImageSourceCreateThumbnailAtIndex(src, 0, thumbOptions as CFDictionary)
    else { throw EngineError(message: "cannot decode image: \(path)") }

    let width = cg.width
    let height = cg.height
    guard width > 2, height > 2 else { throw EngineError(message: "image too small: \(path)") }

    var pixels = [UInt8](repeating: 0, count: width * height)
    let colorSpace = CGColorSpaceCreateDeviceGray()
    guard
        let ctx = CGContext(
            data: &pixels, width: width, height: height, bitsPerComponent: 8,
            bytesPerRow: width, space: colorSpace,
            bitmapInfo: CGImageAlphaInfo.none.rawValue)
    else { throw EngineError(message: "cannot rasterize: \(path)") }
    ctx.draw(cg, in: CGRect(x: 0, y: 0, width: width, height: height))

    // Laplacian kernel, 4-neighbour cross, with wrap-around at the edges.
    var sum = 0.0
    var sumSquares = 0.0
    var n = 0.0
    for y in 1..<(height - 1) {
        for x in 1..<(width - 1) {
            let i = y * width + x
            let laplacian = Double(4) * Double(pixels[i])
                - Double(pixels[i - 1]) - Double(pixels[i + 1])
                - Double(pixels[i - width]) - Double(pixels[i + width])
            sum += laplacian
            sumSquares += laplacian * laplacian
            n += 1
        }
    }
    guard n > 0 else { throw EngineError(message: "empty luma image: \(path)") }
    let mean = sum / n
    return sumSquares / n - mean * mean
}

/// Quality is a small vector of cheap, explainable signals. No model weights,
/// so it runs anywhere and stays inspectable.
func quality(path: String, blurThreshold: Double) throws -> [String: Any] {
    var out: [String: Any] = ["path": path]
    let sharpness = try sharpnessScore(path: path, maxPx: 512)
    out["sharpness"] = sharpness
    out["blurred"] = sharpness < blurThreshold

    if let src = CGImageSourceCreateWithURL(
        URL(fileURLWithPath: path) as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
        let props = CGImageSourceCopyPropertiesAtIndex(src, 0, nil) as? [CFString: Any]
    {
        let w = (props[kCGImagePropertyPixelWidth] as? NSNumber)?.intValue ?? 0
        let h = (props[kCGImagePropertyPixelHeight] as? NSNumber)?.intValue ?? 0
        out["width"] = w
        out["height"] = h
        if w > 0 && h > 0 {
            out["megapixels"] = Double(w * h) / 1_000_000.0
        }
    }
    let attrs = try? FileManager.default.attributesOfItem(atPath: path)
    let bytes = (attrs?[.size] as? NSNumber)?.int64Value ?? 0
    out["sizeBytes"] = bytes
    // Bytes-per-pixel separates a heavily compressed photo from a detailed one.
    if let mp = out["megapixels"] as? Double, mp > 0 {
        out["bytesPerPixel"] = Double(bytes) / (mp * 1_000_000.0)
    }
    return out
}

/// Scene tags from the built-in Vision classifier (mountain, flower, bird…).
/// On-device, no model download: ~1000 everyday categories, one pass.
func classifyScene(path: String, top: Int) throws -> [String: Any] {
    guard let src = CGImageSourceCreateWithURL(
        URL(fileURLWithPath: path) as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
        let cg = CGImageSourceCreateImageAtIndex(src, 0, nil)
    else { throw EngineError(message: "not an image: \(path)") }
    let req = VNClassifyImageRequest()
    do {
        try VNImageRequestHandler(cgImage: cg, options: [:]).perform([req])
    } catch {
        throw EngineError(message: "classify failed: \(error)")
    }
    let tags = (req.results ?? []).prefix(max(1, top)).map { obs -> [String: Any] in
        ["tag": obs.identifier, "confidence": Double(obs.confidence)]
    }
    return ["path": path, "tags": tags]
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

/// Runs AVFoundation-touching work on a background thread and blocks the
/// caller for the result. Required because the synchronous track/reader APIs
/// deadlock on macOS 26 when the fig inspector's callbacks need the calling
/// thread's queue (sampled: `tracks(withMediaType:)` wedged in
/// `_dispatch_group_wait` from main). The main thread stays free to service
/// those callbacks while `done` is waited on here.
func background<T>(_ work: @escaping () throws -> T) throws -> T {
    var boxed: Result<T, Error>!
    let done = DispatchSemaphore(value: 0)
    DispatchQueue.global(qos: .userInitiated).async {
        boxed = Result(catching: work)
        done.signal()
    }
    // The engine's read loop runs on the main thread, so a plain wait()
    // would freeze it — and AVFoundation posts callbacks that need the main
    // queue/run loop to drain (the sampled macOS 26 `tracks(withMediaType:)`
    // wedge). Pump the current (main) run loop while waiting instead.
    while done.wait(timeout: .now() + 0.05) == .timedOut {
        _ = RunLoop.current.run(mode: .default, before: Date(timeIntervalSinceNow: 0.02))
    }
    return try boxed.get()
}

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

    case "hash_batch":
        guard let paths = params["paths"] as? [String], !paths.isEmpty else {
            throw EngineError(message: "hash_batch requires a non-empty 'paths' array")
        }
        if paths.count > 500 {
            throw EngineError(message: "hash_batch accepts at most 500 paths, got \(paths.count)")
        }
        return hashBatch(paths: paths)

    case "dhash_batch":
        guard let paths = params["paths"] as? [String], !paths.isEmpty else {
            throw EngineError(message: "dhash_batch requires a non-empty 'paths' array")
        }
        if paths.count > 500 {
            throw EngineError(message: "dhash_batch accepts at most 500 paths, got \(paths.count)")
        }
        return dHashBatch(paths: paths)

    case "embed":
        guard let path = str("path") else { throw EngineError(message: "embed requires 'path'") }
        return try embedImage(path: path)

    case "embed_batch":
        guard let paths = params["paths"] as? [String], !paths.isEmpty else {
            throw EngineError(message: "embed_batch requires a non-empty 'paths' array")
        }
        if paths.count > 500 {
            throw EngineError(message: "embed_batch accepts at most 500 paths, got \(paths.count)")
        }
        return embedBatch(paths: paths)

    case "clip_encode_text":
        guard let text = str("text"), let modelDir = str("modelDir") else {
            throw EngineError(message: "clip_encode_text requires 'text' and 'modelDir'")
        }
        return try background { try clipEncodeText(text, modelDir: modelDir) }

    case "clip_encode_image":
        guard let path = str("path"), let modelDir = str("modelDir") else {
            throw EngineError(message: "clip_encode_image requires 'path' and 'modelDir'")
        }
        return try background { try clipEncodeImage(path: path, modelDir: modelDir) }

    case "clip_keyframes":
        guard let path = str("path"), let modelDir = str("modelDir") else {
            throw EngineError(message: "clip_keyframes requires 'path' and 'modelDir'")
        }
        return try background {
            try clipKeyframes(path: path, modelDir: modelDir,
                every: dbl("everySeconds", 2.0), maxFrames: int("maxFrames", 12))
        }

    case "faces":
        guard let path = str("path") else { throw EngineError(message: "faces requires 'path'") }
        return try detectFaces(path: path)

    case "detect_animals":
        guard let path = str("path") else { throw EngineError(message: "detect_animals requires 'path'") }
        return try detectAnimals(path: path)

    case "cluster_faces":
        guard let paths = params["paths"] as? [String], !paths.isEmpty else {
            throw EngineError(message: "cluster_faces requires a non-empty 'paths' array")
        }
        if paths.count > 500 {
            throw EngineError(message: "cluster_faces accepts at most 500 paths, got \(paths.count)")
        }
        return try clusterFaces(paths: paths, threshold: dbl("threshold", 0.55))

    case "quality":
        guard let path = str("path") else { throw EngineError(message: "quality requires 'path'") }
        return try quality(path: path, blurThreshold: dbl("blurThreshold", 25.0))

    case "classify":
        guard let path = str("path") else { throw EngineError(message: "classify requires 'path'") }
        return try classifyScene(path: path, top: int("top", 3))

    case "thumb":
        guard let path = str("path") else { throw EngineError(message: "thumb requires 'path'") }
        return try thumbnail(
            path: path, maxPx: int("maxPx", 512), quality: dbl("quality", 0.72))

    case "compress":
        guard let path = str("path") else { throw EngineError(message: "compress requires 'path'") }
        guard let output = str("output") else { throw EngineError(message: "compress requires 'output'") }
        return try compressImage(path: path, output: output, maxPx: int("maxPx", 1280), quality: dbl("quality", 0.72))

    case "compress_video":
        guard let path = str("path") else { throw EngineError(message: "compress_video requires 'path'") }
        guard let output = str("output") else { throw EngineError(message: "compress_video requires 'output'") }
        return try background { try compressVideo(path: path, output: output) }

    case "strip_copy":
        guard let path = str("path") else { throw EngineError(message: "strip_copy requires 'path'") }
        guard let output = str("output") else { throw EngineError(message: "strip_copy requires 'output'") }
        return try stripCopy(path: path, output: output)

    case "make_highlight":
        guard let output = str("output") else { throw EngineError(message: "make_highlight requires 'output'") }
        let maxSeconds = dbl("maxSeconds", 60)
        // Story clips (path + optional day-boundary title); legacy "paths"
        // arrays still accepted and render untitled.
        var inputs: [(path: String, title: String?)] = []
        if let arr = params["clips"] as? [[String: Any]] {
            for d in arr {
                if let p = d["path"] as? String {
                    inputs.append((path: p, title: d["title"] as? String))
                }
            }
        } else if let paths = params["paths"] as? [String] {
            inputs = paths.map { (path: $0, title: nil) }
        }
        guard !inputs.isEmpty else {
            throw EngineError(message: "make_highlight requires 'clips' or a non-empty 'paths' array")
        }
        return try background { try makeHighlight(inputs: inputs, output: output, maxSeconds: maxSeconds) }

    case "face_thumb":
        guard let path = str("path") else { throw EngineError(message: "face_thumb requires 'path'") }
        return ["image": try faceThumbnail(
            path: path,
            x: dbl("x", 0), y: dbl("y", 0),
            width: dbl("width", 0), height: dbl("height", 0),
            maxPx: int("maxPx", 256))]

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
