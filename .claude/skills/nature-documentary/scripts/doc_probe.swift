import AVFoundation
import CoreGraphics
import CoreMedia
import Vision
import AppKit

setvbuf(stdout, nil, _IONBF, 0)

// Throwaway probe: make_documentary (9:16 + naturalFirst + endCard) against
// the real engine binary with synthetic media. Asserts canvas size, audio
// band separation (camera 220Hz rides alone; music 880Hz only under silent
// segments), and OCR-reads the closing stats card.

let dir = "/tmp/docprobe"
let engine = "/Users/yasihusain/Projects/fotoflow/src-tauri/binaries/engine-aarch64-apple-darwin"
var failures: [String] = []
func check(_ cond: Bool, _ label: String) {
    print(cond ? "PASS: \(label)" : "FAIL: \(label)")
    if !cond { failures.append(label) }
}

func solidJPEG(_ path: String, _ color: (CGFloat, CGFloat, CGFloat)) throws {
    let w = 1920, h = 1080
    let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)!
    ctx.setFillColor(CGColor(red: color.0, green: color.1, blue: color.2, alpha: 1))
    ctx.fill(CGRect(x: 0, y: 0, width: w, height: h))
    let img = ctx.makeImage()!
    let dest = CGImageDestinationCreateWithURL(URL(fileURLWithPath: path) as CFURL, "public.jpeg" as CFString, 1, nil)!
    CGImageDestinationAddImage(dest, img, nil)
    CGImageDestinationFinalize(dest)
}

// 3s red clip, video-only (audio added separately by muxing a WAV — an
// in-writer audio input wedges video on macOS 26, probed in AGENTS.md).
func makeClip(_ path: String) throws {
    let w = 640, h = 360, fps = 30, seconds = 3.0
    let url = URL(fileURLWithPath: path)
    try? FileManager.default.removeItem(at: url)
    let writer = try AVAssetWriter(outputURL: url, fileType: .mp4)
    let vIn = AVAssetWriterInput(mediaType: .video, outputSettings: [
        AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: w, AVVideoHeightKey: h])
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: vIn, sourcePixelBufferAttributes: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        kCVPixelBufferWidthKey as String: w, kCVPixelBufferHeightKey as String: h])
    writer.add(vIn)
    writer.startWriting(); writer.startSession(atSourceTime: .zero)
    for f in 0..<Int(seconds * Double(fps)) {
        var spins = 0
        while !vIn.isReadyForMoreMediaData {
            usleep(2000)
            spins += 1
            if spins > 15000 { throw NSError(domain: "probe", code: 5) }
        }
        var pb: CVPixelBuffer?
        if let pool = adaptor.pixelBufferPool {
            CVPixelBufferPoolCreatePixelBuffer(nil, pool, &pb)
        } else {
            CVPixelBufferCreate(nil, w, h, kCVPixelFormatType_32BGRA, nil, &pb)
        }
        guard let buf = pb else { throw NSError(domain: "probe", code: 1) }
        CVPixelBufferLockBaseAddress(buf, [])
        let bpr = CVPixelBufferGetBytesPerRow(buf)
        let base = CVPixelBufferGetBaseAddress(buf)!
        for row in 0..<h {
            let p = base.advanced(by: row * bpr).assumingMemoryBound(to: UInt8.self)
            for col in 0..<w {
                p[col * 4] = 255; p[col * 4 + 1] = 20; p[col * 4 + 2] = 20; p[col * 4 + 3] = 255
            }
        }
        CVPixelBufferUnlockBaseAddress(buf, [])
        adaptor.append(buf, withPresentationTime: CMTime(value: CMTimeValue(f), timescale: CMTimeScale(fps)))
    }
    vIn.markAsFinished()
    let sem = DispatchSemaphore(value: 0)
    writer.finishWriting { sem.signal() }
    sem.wait()
    guard writer.status == .completed else {
        throw NSError(domain: "probe", code: 2, userInfo: [NSLocalizedDescriptionKey: "clip writer \(writer.status.rawValue): \(String(describing: writer.error))"])
    }
}

// int16 WAV sine, amp 0.5.
func makeWAV(_ path: String, seconds: Double, freq: Double) throws {
    let sr = 44100.0
    let total = Int(sr * seconds)
    var pcm = Data()
    for i in 0..<total {
        let v = Int16(0.5 * 32767.0 * sin(2.0 * Double.pi * freq * Double(i) / sr))
        withUnsafeBytes(of: v.littleEndian) { pcm.append(contentsOf: $0) }
    }
    var data = Data()
    func str(_ s: String) { data.append(s.data(using: .ascii)!) }
    func u32(_ v: UInt32) { withUnsafeBytes(of: v.littleEndian) { data.append(contentsOf: $0) } }
    func u16(_ v: UInt16) { withUnsafeBytes(of: v.littleEndian) { data.append(contentsOf: $0) } }
    str("RIFF"); u32(UInt32(36 + pcm.count)); str("WAVE")
    str("fmt "); u32(16); u16(1); u16(1); u32(UInt32(sr)); u32(UInt32(sr * 2)); u16(2); u16(16)
    str("data"); u32(UInt32(pcm.count)); data.append(pcm)
    try data.write(to: URL(fileURLWithPath: path))
}

// Full mono float32 PCM of a media file via ExtAudioFile (the proven path).
func readPCM(_ path: String) -> (samples: [Float], sr: Double) {
    var fref: ExtAudioFileRef?
    guard ExtAudioFileOpenURL(URL(fileURLWithPath: path) as CFURL, &fref) == noErr, let f = fref else {
        return ([], 44100)
    }
    defer { ExtAudioFileDispose(f) }
    var asbd = AudioStreamBasicDescription(mSampleRate: 44100, mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked, mBytesPerPacket: 4,
        mFramesPerPacket: 1, mBytesPerFrame: 4, mChannelsPerFrame: 1, mBitsPerChannel: 32, mReserved: 0)
    ExtAudioFileSetProperty(f, kExtAudioFileProperty_ClientDataFormat,
        UInt32(MemoryLayout<AudioStreamBasicDescription>.size), &asbd)
    var out = [Float]()
    let chunk: UInt32 = 65536
    let buf = malloc(Int(chunk) * 4)!
    defer { free(buf) }
    while true {
        var abl = AudioBufferList(mNumberBuffers: 1, mBuffers: AudioBuffer(
            mNumberChannels: 1, mDataByteSize: chunk * 4, mData: buf))
        var n = chunk
        guard ExtAudioFileRead(f, &n, &abl) == noErr, n > 0 else { break }
        out.append(contentsOf: UnsafeBufferPointer(start: buf.assumingMemoryBound(to: Float.self), count: Int(n)))
    }
    return (out, 44100)
}

// Goertzel power at f over a window.
func bandPower(_ x: [Float], sr: Double, f: Double, from: Double, to: Double) -> Double {
    let a = max(0, Int(from * sr)), b = min(x.count, Int(to * sr))
    guard b > a else { return 0 }
    var re = 0.0, im = 0.0
    let w = 2.0 * Double.pi * f / sr
    for (i, v) in x[a..<b].enumerated() {
        re += Double(v) * cos(w * Double(i))
        im -= Double(v) * sin(w * Double(i))
    }
    let n = Double(b - a)
    return (re * re + im * im) / (n * n)
}
func rms(_ x: [Float], from: Double, to: Double, sr: Double) -> Double {
    let a = max(0, Int(from * sr)), b = min(x.count, Int(to * sr))
    guard b > a else { return 0 }
    var acc = 0.0
    for v in x[a..<b] { acc += Double(v) * Double(v) }
    return (acc / Double(b - a)).squareRoot()
}

// One engine request; returns the reply dict for that id.
var reqID = 0
var inPipe = Pipe(), outPipe = Pipe()
func engineCall(_ proc: Process, cmd: String, params: [String: Any]) throws -> [String: Any] {
    reqID += 1
    var req = ["id": reqID, "cmd": cmd] as [String: Any]
    req["params"] = params
    let line = try JSONSerialization.data(withJSONObject: req)
    inPipe.fileHandleForWriting.write(line + Data([0x0a]))
    let stdout = outPipe.fileHandleForReading
    while true {
        var bytes = Data()
        while true {
            let d = stdout.availableData
            if d.isEmpty { throw NSError(domain: "probe", code: 3) }
            bytes.append(d)
            if bytes.last == 0x0a { break }
        }
        guard let obj = try JSONSerialization.jsonObject(with: bytes) as? [String: Any],
              let rid = obj["id"] as? Int, rid == reqID,
              obj["result"] != nil || obj["error"] != nil else { continue }
        if let err = obj["error"] as? String { throw NSError(domain: "engine", code: 4, userInfo: [NSLocalizedDescriptionKey: err]) }
        return obj["result"] as? [String: Any] ?? [:]
    }
}

func ocrFrame(_ video: String, at seconds: Double) throws -> String {
    let asset = AVURLAsset(url: URL(fileURLWithPath: video))
    let gen = AVAssetImageGenerator(asset: asset)
    gen.appliesPreferredTrackTransform = true
    let cg = try gen.copyCGImage(at: CMTime(seconds: seconds, preferredTimescale: 600), actualTime: nil)
    let req = VNRecognizeTextRequest()
    req.recognitionLevel = .accurate
    try VNImageRequestHandler(cgImage: cg).perform([req])
    return (req.results ?? []).compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
}

// Muxes the video-only clip with the camera WAV via a composition export.
func mux(video: String, wav: String, out: String) throws {
    let comp = AVMutableComposition()
    let vAsset = AVURLAsset(url: URL(fileURLWithPath: video))
    let aAsset = AVURLAsset(url: URL(fileURLWithPath: wav))
    guard let vsrc = vAsset.tracks(withMediaType: .video).first,
          let vdest = comp.addMutableTrack(withMediaType: .video, preferredTrackID: kCMPersistentTrackID_Invalid) else {
        throw NSError(domain: "probe", code: 7)
    }
    try vdest.insertTimeRange(CMTimeRange(start: .zero, duration: vAsset.duration), of: vsrc, at: .zero)
    if let asrc = aAsset.tracks(withMediaType: .audio).first,
       let adest = comp.addMutableTrack(withMediaType: .audio, preferredTrackID: kCMPersistentTrackID_Invalid) {
        try adest.insertTimeRange(CMTimeRange(start: .zero, duration: aAsset.duration), of: asrc, at: .zero)
    }
    guard let session = AVAssetExportSession(asset: comp, presetName: AVAssetExportPresetHighestQuality) else {
        throw NSError(domain: "probe", code: 8)
    }
    try? FileManager.default.removeItem(atPath: out)
    session.outputURL = URL(fileURLWithPath: out)
    session.outputFileType = .mp4
    let sem = DispatchSemaphore(value: 0)
    session.exportAsynchronously { sem.signal() }
    sem.wait()
    guard session.status == .completed else {
        throw NSError(domain: "probe", code: 6, userInfo: [NSLocalizedDescriptionKey: "mux \(session.status.rawValue): \(String(describing: session.error))"])
    }
    print("MEDIA: mux ok")
}

try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
print("MEDIA: making assets")
try solidJPEG("\(dir)/still-green.jpg", (0.1, 0.8, 0.2))
try solidJPEG("\(dir)/still-blue.jpg", (0.15, 0.3, 0.9))
try makeClip("\(dir)/clip-video.mp4")
try makeWAV("\(dir)/cam.wav", seconds: 3, freq: 220)
try makeWAV("\(dir)/music.wav", seconds: 12, freq: 880)
try mux(video: "\(dir)/clip-video.mp4", wav: "\(dir)/cam.wav", out: "\(dir)/clip-cam.mp4")
print("MEDIA: ok")

let proc = Process()
proc.executableURL = URL(fileURLWithPath: engine)
proc.standardInput = inPipe
proc.standardOutput = outPipe
if let errLog = FileHandle(forWritingAtPath: "/tmp/docprobe-engine-stderr.log") {
    proc.standardError = errLog
} else {
    proc.standardError = FileHandle.nullDevice
}
try proc.run()
print("ENGINE: spawned")
defer { proc.terminate() }

// RUN 1: 9:16 + music + endCard. Timeline: green 0-3, clip 3-6, blue 6-9, endCard 9-12.
let clips1: [[String: Any]] = [
    ["path": "\(dir)/still-green.jpg"],
    ["path": "\(dir)/clip-cam.mp4"],
    ["path": "\(dir)/still-blue.jpg"],
]
let out1 = "\(dir)/doc-vertical.mp4"
print("RUN1: sending make_documentary")
let r1 = try engineCall(proc, cmd: "make_documentary", params: [
    "clips": clips1, "output": out1, "maxSeconds": 30.0,
    "aspect": "9:16", "naturalFirst": true,
    "endCard": "Probe Run\n12 photos · 4 videos",
    "musicPath": "\(dir)/music.wav",
])
print("reply1: \(r1)")
check(r1["aspect"] as? String == "9:16", "reply aspect 9:16")
check(r1["endCard"] as? Bool == true, "reply endCard true")
check((r1["videosIncluded"] as? Int) == 1, "video clip included")
check(((r1["audioFrames"] as? Int) ?? 0) > 0, "audio frames written")

print("RUN1: checking output tracks")
let vAsset = AVURLAsset(url: URL(fileURLWithPath: out1))
let vTrack = vAsset.tracks(withMediaType: .video).first
let sz = vTrack?.naturalSize ?? .zero
check(sz.width == 1080 && sz.height == 1920, "vertical canvas 1080x1920 (got \(sz))")

let (pcm, sr) = readPCM(out1)
let music = bandPower(pcm, sr: sr, f: 880, from: 1.0, to: 2.5)
let camUnderMusic = bandPower(pcm, sr: sr, f: 220, from: 1.0, to: 2.5)
check(music > camUnderMusic * 100.0, "music under still (880 >> 220): 880=\(music) 220=\(camUnderMusic)")
let cam = bandPower(pcm, sr: sr, f: 220, from: 3.6, to: 5.4)
let musicUnderCam = bandPower(pcm, sr: sr, f: 880, from: 3.6, to: 5.4)
check(cam > musicUnderCam * 100.0, "natural-first camera alone (220 >> 880): 220=\(cam) 880=\(musicUnderCam)")
let camRMS = rms(pcm, from: 3.6, to: 5.4, sr: sr)
check(abs(camRMS - 0.4 / 2.0.squareRoot()) < 0.08, "camera at native level, not ducked (rms=\(camRMS))")
let outro = bandPower(pcm, sr: sr, f: 880, from: 10.0, to: 11.5)
let outroCam = bandPower(pcm, sr: sr, f: 220, from: 10.0, to: 11.5)
check(outro > outroCam * 100.0, "music outro under end card: 880=\(outro) vs 220=\(outroCam)")

let ocr1 = try ocrFrame(out1, at: 10.5)
print("ocr1: \(ocr1)")
check(ocr1.contains("12") && ocr1.contains("photos"), "end card OCR reads stats")

// RUN 2: 16:9, no music, endCard → camera audio present, no music anywhere.
let out2 = "\(dir)/doc-horizontal.mp4"
let r2 = try engineCall(proc, cmd: "make_documentary", params: [
    "clips": [["path": "\(dir)/still-green.jpg"], ["path": "\(dir)/clip-cam.mp4"]],
    "output": out2, "maxSeconds": 30.0, "aspect": "16:9", "naturalFirst": true,
    "endCard": "Probe Run B\n7 days · 3 places",
])
print("reply2: \(r2)")
check(r2["aspect"] as? String == "16:9", "reply aspect 16:9")
let sz2 = AVURLAsset(url: URL(fileURLWithPath: out2)).tracks(withMediaType: .video).first?.naturalSize ?? .zero
check(sz2.width == 1920 && sz2.height == 1080, "horizontal canvas 1920x1080 (got \(sz2))")
let (pcm2, sr2) = readPCM(out2)
let cam2 = bandPower(pcm2, sr: sr2, f: 220, from: 3.6, to: 5.4)
let nomusic = bandPower(pcm2, sr: sr2, f: 880, from: 0.5, to: 5.4)
check(cam2 > 0.0001, "camera audio present without music bed (220=\(cam2))")
check(nomusic < 0.0001, "no music leaked (880=\(nomusic))")
let ocr2 = try ocrFrame(out2, at: 8.5)
print("ocr2: \(ocr2)")
check(ocr2.contains("places") || ocr2.contains("days"), "horizontal end card OCR reads stats")

if failures.isEmpty {
    print("ALL PASS")
} else {
    print("\(failures.count) FAILURES")
    exit(1)
}
