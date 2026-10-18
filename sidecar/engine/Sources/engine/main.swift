import AVFoundation
import CoreGraphics
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

/// Clusters faces across a set of images into "people" using Vision's own
/// feature-print distance. Needs no downloaded model weights: VNGenerateImageFeaturePrintRequest
/// is part of the OS. Returns one cluster per detected person; singletons are
/// groups of size 1.
func clusterFaces(paths: [String], threshold: Double) throws -> [String: Any] {
    struct Face { var path: String; var box: CGRect; var confidence: Double; var desc: FeaturePrintObservation }
    var entries: [Face] = []
    var unreadable = 0

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
                  let desc = fpRequest.results?.first as? FeaturePrintObservation
            else { continue }
            entries.append(Face(path: path, box: b, confidence: Double(obs.confidence), desc: desc))
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

/// Builds a highlight slideshow MP4 from photos with AVAssetWriter.
///
/// Still images expose zero AVAssetTrack instances on the macOS 26 SDK, so a
/// composition-based export cannot include them; frames are synthesized
/// directly instead (3s per photo at 30fps, H.264). Video clips are counted and
/// skipped for now — mixing their compressed samples with synthesized frames
/// needs format-normalization work that lands in a follow-up.
func makeHighlight(paths: [String], output: String, maxSeconds: Double) throws -> [String: Any] {
    var photos: [CGImage] = []
    var videosSkipped = 0
    for path in paths {
        let url = URL(fileURLWithPath: path)
        if let src = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
           let cg = CGImageSourceCreateImageAtIndex(src, 0, nil) {
            photos.append(cg)
        } else {
            videosSkipped += 1
        }
    }
    guard !photos.isEmpty else { throw EngineError(message: "no decodable photos for highlight") }

    // Output canvas: 1920x1080 letterboxed (16:9 works for WhatsApp and Drive).
    let outW = 1920, outH = 1080
    let fps = 30
    let secondsPerPhoto = 3.0
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
    var appended = 0
    for image in photos {
        for _ in 0..<framesPerPhoto {
            while !vInput.isReadyForMoreMediaData { Thread.sleep(forTimeInterval: 0.01) }
            let pb = try makePixelBuffer(image, index: frame)
            let time = CMTime(value: CMTimeValue(frame), timescale: CMTimeScale(fps))
            if adaptor.append(pb, withPresentationTime: time) {
                appended += 1
            }
            frame += 1
        }
    }
    vInput.markAsFinished()
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
        "seconds": Double(appended) / Double(fps),
        "photos": photos.count,
        "videosSkipped": videosSkipped,
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

    case "hash_batch":
        guard let paths = params["paths"] as? [String], !paths.isEmpty else {
            throw EngineError(message: "hash_batch requires a non-empty 'paths' array")
        }
        if paths.count > 500 {
            throw EngineError(message: "hash_batch accepts at most 500 paths, got \(paths.count)")
        }
        return hashBatch(paths: paths)

    case "faces":
        guard let path = str("path") else { throw EngineError(message: "faces requires 'path'") }
        return try detectFaces(path: path)

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

    case "thumb":
        guard let path = str("path") else { throw EngineError(message: "thumb requires 'path'") }
        return try thumbnail(
            path: path, maxPx: int("maxPx", 512), quality: dbl("quality", 0.72))

    case "compress":
        guard let path = str("path") else { throw EngineError(message: "compress requires 'path'") }
        guard let output = str("output") else { throw EngineError(message: "compress requires 'output'") }
        return try compressImage(path: path, output: output, maxPx: int("maxPx", 1280), quality: dbl("quality", 0.72))

    case "make_highlight":
        guard let paths = params["paths"] as? [String], !paths.isEmpty else {
            throw EngineError(message: "make_highlight requires a non-empty 'paths' array")
        }
        guard let output = str("output") else { throw EngineError(message: "make_highlight requires 'output'") }
        return try makeHighlight(paths: paths, output: output, maxSeconds: dbl("maxSeconds", 60))

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
