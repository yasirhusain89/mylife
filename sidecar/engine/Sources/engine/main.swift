import AVFoundation
import CoreGraphics
import CoreImage
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

/// Builds a highlight MP4 from photos and video clips with AVAssetWriter.
///
/// Still images expose zero AVAssetTrack instances on the macOS 26 SDK, so a
/// composition-based export cannot include them; photo frames are synthesized
/// directly instead (3s per photo at 30fps, H.264). Video clips are decoded
/// with AVAssetReader and their frames drawn onto the same 1920x1080 canvas
/// (up to 3s per clip); a clip's camera audio rides along on an AAC track —
/// original audio only, never added music. The whole timeline is capped at
/// maxSeconds. Files that decode as neither photo nor video count as skipped.
func makeHighlight(paths: [String], output: String, maxSeconds: Double) throws -> [String: Any] {
    enum Clip { case photo(CGImage); case video(String) }
    var clips: [Clip] = []
    var skipped = 0
    for path in paths {
        let url = URL(fileURLWithPath: path)
        if let src = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
           let cg = CGImageSourceCreateImageAtIndex(src, 0, nil) {
            clips.append(.photo(cg))
            continue
        }
        let asset = AVURLAsset(url: url)
        if asset.tracks(withMediaType: .video).first != nil {
            clips.append(.video(path))
        } else {
            skipped += 1
        }
    }
    guard !clips.isEmpty else { throw EngineError(message: "no decodable photos or videos for highlight") }
    let cap = max(1.0, maxSeconds)

    // Output canvas: 1920x1080 letterboxed (16:9 works for WhatsApp and Drive).
    let outW = 1920, outH = 1080
    let fps = 30
    let secondsPerPhoto = 3.0
    let secondsPerClip = 3.0
    let framesPerPhoto = Int(secondsPerPhoto * Double(fps))

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

    // Audio rides along only when at least one clip actually has an audio track.
    var aInput: AVAssetWriterInput?
    for clip in clips {
        if case .video(let path) = clip,
           AVURLAsset(url: URL(fileURLWithPath: path)).tracks(withMediaType: .audio).first != nil {
            let ai = AVAssetWriterInput(mediaType: .audio, outputSettings: [
                AVFormatIDKey: kAudioFormatMPEG4AAC,
                AVSampleRateKey: 44100,
                AVNumberOfChannelsKey: 2,
                AVEncoderBitRateKey: 128_000,
            ])
            ai.expectsMediaDataInRealTime = false
            guard writer.canAdd(ai) else { throw EngineError(message: "cannot add audio input") }
            writer.add(ai)
            aInput = ai
            break
        }
    }

    guard writer.startWriting() else {
        throw EngineError(message: "writer start failed: \(writer.error.map(String.init(describing:)) ?? "unknown")")
    }
    writer.startSession(atSourceTime: .zero)

    func makePixelBuffer(_ image: CGImage, index: Int64) throws -> CVPixelBuffer {
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
        // Aspect-fill
        let scale = max(CGFloat(outW) / CGFloat(image.width), CGFloat(outH) / CGFloat(image.height))
        let drawW = CGFloat(image.width) * scale
        let drawH = CGFloat(image.height) * scale
        ctx.draw(image, in: CGRect(
            x: (CGFloat(outW) - drawW) / 2, y: (CGFloat(outH) - drawH) / 2,
            width: drawW, height: drawH))
        _ = index
        return pb
    }

    var frame: Int64 = 0
    var photos = 0
    var videosIncluded = 0
    var cursor = CMTime.zero
    let ciCtx = CIContext()

    // Center-cropped CGImage covering the 16:9 canvas from a decoded frame.
    func canvasCrop(_ sample: CVPixelBuffer) -> CGImage? {
        let ci = CIImage(cvPixelBuffer: sample)
        let w = ci.extent.width, h = ci.extent.height
        guard w > 0, h > 0 else { return nil }
        let scale = max(CGFloat(outW) / w, CGFloat(outH) / h)
        let crop = CGRect(
            x: (w - CGFloat(outW) / scale) / 2, y: (h - CGFloat(outH) / scale) / 2,
            width: CGFloat(outW) / scale, height: CGFloat(outH) / scale)
        return ciCtx.createCGImage(ci, from: crop)
    }

    // Streams one video clip (up to `take` seconds) onto the timeline:
    // decoded frames drawn on the canvas, camera audio re-stamped after.
    // Returns false when the clip contributed no frames.
    func appendClip(path: String, take: Double) throws -> Bool {
        let asset = AVURLAsset(url: URL(fileURLWithPath: path))
        guard let vTrack = asset.tracks(withMediaType: .video).first else { return false }

        // Audio first: a few seconds buffer cheaply; a failed audio read
        // degrades to silent video instead of failing the clip.
        var audioSamples: [CMSampleBuffer] = []
        if let aInput, let aTrack = asset.tracks(withMediaType: .audio).first,
           let aReader = try? AVAssetReader(asset: asset) {
            let aOut = AVAssetReaderTrackOutput(track: aTrack, outputSettings: nil)
            aReader.add(aOut)
            if aReader.startReading() {
                while let sb = aOut.copyNextSampleBuffer() {
                    let pts = CMSampleBufferGetPresentationTimeStamp(sb)
                    if pts.seconds < 0 { continue } // encoder priming, not content
                    if pts.seconds >= take { break }
                    var timing = CMSampleTimingInfo(
                        duration: CMSampleBufferGetDuration(sb),
                        presentationTimeStamp: CMTimeAdd(cursor, pts),
                        decodeTimeStamp: .invalid)
                    var fresh: CMSampleBuffer?
                    if CMSampleBufferCreateCopyWithNewTiming(
                        allocator: kCFAllocatorDefault, sampleBuffer: sb,
                        sampleTimingEntryCount: 1, sampleTimingArray: &timing,
                        sampleBufferOut: &fresh) == noErr, let f = fresh {
                        audioSamples.append(f)
                    }
                }
            }
        }

        let vReader = try AVAssetReader(asset: asset)
        let vOut = AVAssetReaderTrackOutput(track: vTrack, outputSettings: [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        ])
        vReader.add(vOut)
        guard vReader.startReading() else { return false }
        var wroteAny = false
            while let sb = vOut.copyNextSampleBuffer() {
                let pts = CMSampleBufferGetPresentationTimeStamp(sb)
                if pts.seconds >= take { break }
                guard let pb = CMSampleBufferGetImageBuffer(sb),
                      let cg = canvasCrop(pb) else { continue }
                // Native sample timing (no re-quantization): dense source
                // framerates would otherwise collapse onto identical stamps
                // and every append would fail.
                let t = CMTimeAdd(cursor, pts)
            while !vInput.isReadyForMoreMediaData { Thread.sleep(forTimeInterval: 0.01) }
                let out = try makePixelBuffer(cg, index: frame)
                adaptor.append(out, withPresentationTime: t)
                frame += 1
                wroteAny = true
        }
        // Video then audio per clip; the writer multiplexes by PTS at finish.
        if let aInput {
            for sb in audioSamples {
                while !aInput.isReadyForMoreMediaData { Thread.sleep(forTimeInterval: 0.01) }
                aInput.append(sb)
            }
        }
        return wroteAny
    }

    for clip in clips {
        if cursor.seconds >= cap { break }
        switch clip {
        case .photo(let image):
            let budget = Int((cap - cursor.seconds) * Double(fps))
            for _ in 0..<min(framesPerPhoto, budget) {
                while !vInput.isReadyForMoreMediaData { Thread.sleep(forTimeInterval: 0.01) }
                let pb = try makePixelBuffer(image, index: frame)
                adaptor.append(pb, withPresentationTime: cursor)
                frame += 1
                cursor = CMTimeAdd(cursor, CMTime(value: 1, timescale: CMTimeScale(fps)))
            }
            photos += 1
        case .video(let path):
            let take = min(secondsPerClip, cap - cursor.seconds)
            do {
                if try appendClip(path: path, take: take) {
                    videosIncluded += 1
                } else {
                    skipped += 1
                }
            } catch {
                skipped += 1
            }
            cursor = CMTimeAdd(cursor, CMTime(seconds: take, preferredTimescale: CMTimeScale(fps)))
        }
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
        "videosIncluded": videosIncluded,
        "videosSkipped": skipped,
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
    done.wait()
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

    case "make_highlight":
        guard let paths = params["paths"] as? [String], !paths.isEmpty else {
            throw EngineError(message: "make_highlight requires a non-empty 'paths' array")
        }
        guard let output = str("output") else { throw EngineError(message: "make_highlight requires 'output'") }
        let maxSeconds = dbl("maxSeconds", 60)
        return try background { try makeHighlight(paths: paths, output: output, maxSeconds: maxSeconds) }

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
