import MetalKit
import Metal
import Foundation
import UIKit

struct Uniforms {
    var bl: simd_float2
    var br: simd_float2
    var tl: simd_float2
    var tr: simd_float2
    var screenSize: simd_float2
    var opacity: Float
    var layerMode: Float
    var valStops = simd_float8()
    var colStops = simd_float4x4() // Pack 8 colors as two 4x4 matrices
    var colStops2 = simd_float4x4()
}

class WindDataCacheItem: NSObject {
    let texture: MTLTexture
    let bytes: [UInt8]
    let width: Int
    let height: Int
    init(texture: MTLTexture, bytes: [UInt8], width: Int, height: Int) {
        self.texture = texture
        self.bytes = bytes
        self.width = width
        self.height = height
    }
}

class RadarMetalOverlay: MTKView, MTKViewDelegate {
    
    private let projectionLock = NSLock()
    private let windDataLock = NSLock()
    private var syncBl = simd_float2(0, 0)
    private var syncBr = simd_float2(0, 0)
    private var syncTl = simd_float2(0, 0)
    private var syncTr = simd_float2(0, 0)
    
    func updateProjection(bl: simd_float2, br: simd_float2, tl: simd_float2, tr: simd_float2) {
        projectionLock.lock()
        syncBl = bl
        syncBr = br
        syncTl = tl
        syncTr = tr
        projectionLock.unlock()
        
        DispatchQueue.main.async {
            self.setNeedsDisplay()
        }
    }
    var opacity: Float = 0.7
    var layerMode: Float = 0.0
    var valStops = [Float](repeating: 0.0, count: 8)
    var colStops = [simd_float4](repeating: simd_float4(0, 0, 0, 0), count: 8)
    
    private var commandQueue: MTLCommandQueue?
    private var pipelineState: MTLRenderPipelineState?
    private var activeTexture: MTLTexture?
    private var pendingTexture: MTLTexture?
    private var samplerState: MTLSamplerState?
    
    private let textureCache = NSCache<NSString, AnyObject>()
    private var activeTasks: [String: URLSessionDataTask] = [:]
    private let taskLock = NSLock()
    
    private var vertexBuffer: MTLBuffer?
    
    // Wind Simulation
    private let numParticles = 2000
    private var particles: [Particle] = []
    private var windBytes: [UInt8] = []
    private var windWidth = 0
    private var windHeight = 0
    private var lastFrameTime = Date()
    private var particlePipelineState: MTLRenderPipelineState?
    
    private var activeRequestSeq = 0

    
    class Particle {
        var x = Float.random(in: 0...1)
        var y = Float.random(in: 0...1)
        var age = Float.random(in: 0...0.8)
        var lifetime = Float.random(in: 6.6...13.3)
        var trailX = [Float](repeating: 0, count: 24)
        var trailY = [Float](repeating: 0, count: 24)
        var trailCount = 0
        var updateCount = 0
    }
    
    init(frame: CGRect, device: MTLDevice) {
        super.init(frame: frame, device: device)
        self.delegate = self
        self.backgroundColor = .clear
        self.isOpaque = false
        self.layer.isOpaque = false
        self.isPaused = true
        self.enableSetNeedsDisplay = true
        textureCache.countLimit = 120
        
        setupMetal()
        setupParticles()
    }
    
    required init(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }
    
    private func setupMetal() {
        guard let device = self.device else { return }
        commandQueue = device.makeCommandQueue()
        
        let defaultLibrary = device.makeDefaultLibrary()
        let vertexFunction = defaultLibrary?.makeFunction(name: "vertexMain")
        let fragmentFunction = defaultLibrary?.makeFunction(name: "fragmentMain")
        
        let pipelineStateDescriptor = MTLRenderPipelineDescriptor()
        pipelineStateDescriptor.vertexFunction = vertexFunction
        pipelineStateDescriptor.fragmentFunction = fragmentFunction
        pipelineStateDescriptor.colorAttachments[0].pixelFormat = colorPixelFormat
        
        // Alpha blending configuration
        pipelineStateDescriptor.colorAttachments[0].isBlendingEnabled = true
        pipelineStateDescriptor.colorAttachments[0].sourceRGBBlendFactor = .sourceAlpha
        pipelineStateDescriptor.colorAttachments[0].destinationRGBBlendFactor = .oneMinusSourceAlpha
        pipelineStateDescriptor.colorAttachments[0].rgbBlendOperation = .add
        pipelineStateDescriptor.colorAttachments[0].sourceAlphaBlendFactor = .sourceAlpha
        pipelineStateDescriptor.colorAttachments[0].destinationAlphaBlendFactor = .oneMinusSourceAlpha
        pipelineStateDescriptor.colorAttachments[0].alphaBlendOperation = .add
        
        do {
            pipelineState = try device.makeRenderPipelineState(descriptor: pipelineStateDescriptor)
        } catch {
            print("Failed to create Metal render pipeline state: \(error)")
        }
        
        // Setup particle pipeline state
        if let pVertex = defaultLibrary?.makeFunction(name: "particleVertexMain"),
           let pFragment = defaultLibrary?.makeFunction(name: "particleFragmentMain") {
            let pDesc = MTLRenderPipelineDescriptor()
            pDesc.vertexFunction = pVertex
            pDesc.fragmentFunction = pFragment
            pDesc.colorAttachments[0].pixelFormat = colorPixelFormat
            
            pDesc.colorAttachments[0].isBlendingEnabled = true
            pDesc.colorAttachments[0].sourceRGBBlendFactor = .sourceAlpha
            pDesc.colorAttachments[0].destinationRGBBlendFactor = .one
            pDesc.colorAttachments[0].rgbBlendOperation = .add
            pDesc.colorAttachments[0].sourceAlphaBlendFactor = .sourceAlpha
            pDesc.colorAttachments[0].destinationAlphaBlendFactor = .one
            pDesc.colorAttachments[0].alphaBlendOperation = .add
            
            let vDesc = MTLVertexDescriptor()
            vDesc.attributes[0].format = .float2
            vDesc.attributes[0].offset = 0
            vDesc.attributes[0].bufferIndex = 0
            vDesc.attributes[1].format = .float
            vDesc.attributes[1].offset = MemoryLayout<Float>.size * 2
            vDesc.attributes[1].bufferIndex = 0
            vDesc.layouts[0].stride = MemoryLayout<Float>.size * 3
            pDesc.vertexDescriptor = vDesc
            
            do {
                particlePipelineState = try device.makeRenderPipelineState(descriptor: pDesc)
            } catch {
                print("Failed to create Metal particle render pipeline state: \(error)")
            }
        }
        
        // Quad vertices (BL, BR, TL, TR in unit coordinates 0..1)
        let quadVertices: [Float] = [
            0.0, 0.0,
            1.0, 0.0,
            0.0, 1.0,
            0.0, 1.0,
            1.0, 0.0,
            1.0, 1.0
        ]
        vertexBuffer = device.makeBuffer(bytes: quadVertices, length: quadVertices.count * MemoryLayout<Float>.size, options: [])
        
        // Sampler
        let samplerDesc = MTLSamplerDescriptor()
        samplerDesc.minFilter = .nearest
        samplerDesc.magFilter = .nearest
        samplerState = device.makeSamplerState(descriptor: samplerDesc)
    }
    
    private func setupParticles() {
        for _ in 0..<numParticles {
            particles.append(Particle())
        }
    }
    
    private func trackTask(urlStr: String, task: URLSessionDataTask) {
        taskLock.lock()
        if let existing = activeTasks[urlStr] {
            existing.cancel()
        }
        activeTasks[urlStr] = task
        taskLock.unlock()
    }

    private func untrackTask(urlStr: String) {
        taskLock.lock()
        activeTasks.removeValue(forKey: urlStr)
        taskLock.unlock()
    }

    func cancelStaleRequests(keepUrls: Set<String>) {
        taskLock.lock()
        var urlsToCancel: [String] = []
        for (urlStr, task) in activeTasks {
            if !keepUrls.contains(urlStr) {
                task.cancel()
                urlsToCancel.append(urlStr)
            }
        }
        for urlStr in urlsToCancel {
            activeTasks.removeValue(forKey: urlStr)
        }
        taskLock.unlock()
    }

    func loadTextureAsync(urlStr: String, seq: Int, isWind: Bool = false) {
        if seq < activeRequestSeq { return }
        activeRequestSeq = seq

        // Check Cache first
        if let cached = textureCache.object(forKey: urlStr as NSString) as? WindDataCacheItem {
            if seq >= activeRequestSeq {
                self.pendingTexture = cached.texture
                if layerMode == 3.0 {
                    self.windDataLock.lock()
                    if cached.bytes.isEmpty {
                        // Extract bytes on demand if they are missing (e.g. if cached during prefetch on non-wind layer)
                        let w = cached.texture.width
                        let h = cached.texture.height
                        var bytes = [UInt8](repeating: 0, count: w * h * 4)
                        cached.texture.getBytes(&bytes, bytesPerRow: w * 4, from: MTLRegionMake2D(0, 0, w, h), mipmapLevel: 0)
                        self.windBytes = bytes
                        self.windWidth = w
                        self.windHeight = h
                    } else {
                        self.windBytes = cached.bytes
                        self.windWidth = cached.width
                        self.windHeight = cached.height
                    }
                    self.windDataLock.unlock()
                }
                self.setNeedsDisplay()
            }
            return
        }

        guard let url = URL(string: urlStr) else { return }
        
        let task = URLSession.shared.dataTask(with: url) { [weak self] data, response, error in
            guard let self = self else { return }
            defer { self.untrackTask(urlStr: urlStr) }
            guard let data = data, error == nil else { return }
            
            if let image = UIImage(data: data) {
                let textureLoader = MTKTextureLoader(device: self.device!)
                do {
                    let options: [MTKTextureLoader.Option: Any] = [
                        .premultiplyAlpha: false
                    ]
                    let tex = try textureLoader.newTexture(cgImage: image.cgImage!, options: options)
                    
                    var bytes: [UInt8] = []
                    var w = 0
                    var h = 0
                    if self.layerMode == 3.0 {
                        w = tex.width
                        h = tex.height
                        bytes = [UInt8](repeating: 0, count: w * h * 4)
                        tex.getBytes(&bytes, bytesPerRow: w * 4, from: MTLRegionMake2D(0, 0, w, h), mipmapLevel: 0)
                    }
                    
                    let item = WindDataCacheItem(texture: tex, bytes: bytes, width: w, height: h)
                    self.textureCache.setObject(item, forKey: urlStr as NSString)
                    
                    DispatchQueue.main.async {
                        if seq >= self.activeRequestSeq {
                            self.pendingTexture = tex
                             if self.layerMode == 3.0 {
                                 self.windDataLock.lock()
                                 self.windBytes = bytes
                                 self.windWidth = w
                                 self.windHeight = h
                                 self.windDataLock.unlock()
                             }
                            self.setNeedsDisplay()
                        }
                    }
                } catch {
                    print("Failed to convert image to Metal texture: \(error)")
                }
            }
        }
        trackTask(urlStr: urlStr, task: task)
        task.resume()
    }

    func prefetchTexture(urlStr: String, isWind: Bool = false) {
        if textureCache.object(forKey: urlStr as NSString) != nil { return }
        guard let url = URL(string: urlStr) else { return }

        let task = URLSession.shared.dataTask(with: url) { [weak self] data, response, error in
            guard let self = self else { return }
            defer { self.untrackTask(urlStr: urlStr) }
            guard let data = data, error == nil else { return }
            
            if let image = UIImage(data: data) {
                let textureLoader = MTKTextureLoader(device: self.device!)
                do {
                    let options: [MTKTextureLoader.Option: Any] = [
                        .premultiplyAlpha: false
                    ]
                    let tex = try textureLoader.newTexture(cgImage: image.cgImage!, options: options)
                    
                    var bytes: [UInt8] = []
                    var w = 0
                    var h = 0
                    if self.layerMode == 3.0 {
                        w = tex.width
                        h = tex.height
                        bytes = [UInt8](repeating: 0, count: w * h * 4)
                        tex.getBytes(&bytes, bytesPerRow: w * 4, from: MTLRegionMake2D(0, 0, w, h), mipmapLevel: 0)
                    }
                    
                    let item = WindDataCacheItem(texture: tex, bytes: bytes, width: w, height: h)
                    self.textureCache.setObject(item, forKey: urlStr as NSString)
                } catch {
                    print("Failed to prefetch texture: \(error)")
                }
            }
        }
        trackTask(urlStr: urlStr, task: task)
        task.resume()
    }


    
    func setStops(values: [Float], colors: [simd_float4]) {
        self.valStops = values
        self.colStops = colors
    }
    
    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {}
    
    func draw(in view: MTKView) {
        guard let drawable = currentDrawable,
              let renderPassDescriptor = currentRenderPassDescriptor,
              let pipelineState = pipelineState,
              let commandQueue = commandQueue,
              let vertexBuffer = vertexBuffer else { return }
        
        // Swap textures if a new one is loaded
        if let pending = pendingTexture {
            activeTexture = pending
            pendingTexture = nil
        }
        
        guard let texture = activeTexture else { return }
        
        let commandBuffer = commandQueue.makeCommandBuffer()
        let renderEncoder = commandBuffer?.makeRenderCommandEncoder(descriptor: renderPassDescriptor)
        
        renderEncoder?.setRenderPipelineState(pipelineState)
        renderEncoder?.setVertexBuffer(vertexBuffer, offset: 0, index: 0)
        
        // Safe coordinates copy
        projectionLock.lock()
        let currentBl = syncBl
        let currentBr = syncBr
        let currentTl = syncTl
        let currentTr = syncTr
        projectionLock.unlock()

        // Pack Uniforms struct
        var uniforms = Uniforms(
            bl: currentBl,
            br: currentBr,
            tl: currentTl,
            tr: currentTr,
            screenSize: simd_float2(Float(bounds.width), Float(bounds.height)),
            opacity: opacity,
            layerMode: layerMode
        )
        
        for i in 0..<8 {
            uniforms.valStops[i] = valStops[i]
        }
        
        // Pack colors
        var matrix1 = simd_float4x4()
        var matrix2 = simd_float4x4()
        matrix1[0] = colStops[0]
        matrix1[1] = colStops[1]
        matrix1[2] = colStops[2]
        matrix1[3] = colStops[3]
        matrix2[0] = colStops[4]
        matrix2[1] = colStops[5]
        matrix2[2] = colStops[6]
        matrix2[3] = colStops[7]
        uniforms.colStops = matrix1
        uniforms.colStops2 = matrix2
        
        renderEncoder?.setVertexBytes(&uniforms, length: MemoryLayout<Uniforms>.stride, index: 0)
        renderEncoder?.setFragmentBytes(&uniforms, length: MemoryLayout<Uniforms>.stride, index: 0)
        renderEncoder?.setFragmentTexture(texture, index: 0)
        renderEncoder?.setFragmentSamplerState(samplerState, index: 0)
        
        // Draw weather quad
        renderEncoder?.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: 6)
        
        // Draw wind particles
        if layerMode == 3.0 {
            drawParticles(renderEncoder: renderEncoder!, bl: currentBl, br: currentBr, tl: currentTl, tr: currentTr)
        }
        
        renderEncoder?.endEncoding()
        commandBuffer?.present(drawable)
        commandBuffer?.commit()
    }
    
    private func getWindVelocityAtPixel(x: Int, y: Int, windBytes: [UInt8], windWidth: Int, windHeight: Int) -> (Float, Float) {
        let offset = y * windWidth * 4 + x * 4
        if offset + 3 >= windBytes.count {
            return (0, 0)
        }
        let r = Float(windBytes[offset])
        let g = Float(windBytes[offset + 1])
        let b = Float(windBytes[offset + 2])
        let a = Float(windBytes[offset + 3])
        
        let uRaw = r * 256.0 + g
        let vRaw = b * 256.0 + a
        
        if uRaw >= 65535.0 || vRaw >= 65535.0 || uRaw == 0 || vRaw == 0 {
            return (0, 0)
        }
        
        let u = uRaw / 100.0 - 100.0
        let v = vRaw / 100.0 - 100.0
        return (u, v)
    }

    private func getWindVelocityInterpolated(xNorm: Float, yNorm: Float, windBytes: [UInt8], windWidth: Int, windHeight: Int) -> (Float, Float) {
        if windWidth <= 1 || windHeight <= 1 { return (0, 0) }
        
        let px = xNorm * Float(windWidth - 1)
        let py = (1.0 - yNorm) * Float(windHeight - 1)
        
        let x0 = max(0, min(Int(px), windWidth - 1))
        let y0 = max(0, min(Int(py), windHeight - 1))
        let x1 = max(0, min(x0 + 1, windWidth - 1))
        let y1 = max(0, min(y0 + 1, windHeight - 1))
        
        let tx = px - Float(x0)
        let ty = py - Float(y0)
        
        let p00 = getWindVelocityAtPixel(x: x0, y: y0, windBytes: windBytes, windWidth: windWidth, windHeight: windHeight)
        let p10 = getWindVelocityAtPixel(x: x1, y: y0, windBytes: windBytes, windWidth: windWidth, windHeight: windHeight)
        let p01 = getWindVelocityAtPixel(x: x0, y: y1, windBytes: windBytes, windWidth: windWidth, windHeight: windHeight)
        let p11 = getWindVelocityAtPixel(x: x1, y: y1, windBytes: windBytes, windWidth: windWidth, windHeight: windHeight)
        
        let u0 = p00.0 + tx * (p10.0 - p00.0)
        let u1 = p01.0 + tx * (p11.0 - p01.0)
        let u = u0 + ty * (u1 - u0)
        
        let v0 = p00.1 + tx * (p10.1 - p00.1)
        let v1 = p01.1 + tx * (p11.1 - p01.1)
        let v = v0 + ty * (v1 - v0)
        
        return (u, v)
    }
    
    private func drawParticles(
        renderEncoder: MTLRenderCommandEncoder,
        bl: simd_float2,
        br: simd_float2,
        tl: simd_float2,
        tr: simd_float2
    ) {
        // Safe coordinates copy under lock
        windDataLock.lock()
        let currentWindBytes = self.windBytes
        let currentWindWidth = self.windWidth
        let currentWindHeight = self.windHeight
        windDataLock.unlock()
        
        if currentWindBytes.isEmpty || currentWindWidth == 0 || currentWindHeight == 0 { return }
        
        let now = Date()
        var dt = Float(now.timeIntervalSince(lastFrameTime))
        if dt > 0.1 { dt = 0.1 }
        lastFrameTime = now
        
        for p in particles {
            p.age += dt / p.lifetime
            if p.age >= 1.0 || p.x < 0 || p.x > 1 || p.y < 0 || p.y > 1 {
                resetParticle(p)
                continue
            }
            
            let (u, v) = getWindVelocityInterpolated(
                xNorm: p.x,
                yNorm: p.y,
                windBytes: currentWindBytes,
                windWidth: currentWindWidth,
                windHeight: currentWindHeight
            )
            if u == 0 && v == 0 {
                resetParticle(p)
                continue
            }
            
            let speedFactor: Float = 2.5 * 4.0
            let dxNorm = (u * dt * speedFactor * 1200.0) / 1210000.0
            let dyNorm = (v * dt * speedFactor * 1200.0) / 1310000.0 // Corrected vertical sign (positive)
            
            p.x += dxNorm
            p.y += dyNorm
            
            p.updateCount += 1
            if p.updateCount % 2 == 0 {
                if p.trailCount < 24 {
                    p.trailX[p.trailCount] = p.x
                    p.trailY[p.trailCount] = p.y
                    p.trailCount += 1
                } else {
                    for i in 0..<23 {
                        p.trailX[i] = p.trailX[i + 1]
                        p.trailY[i] = p.trailY[i + 1]
                    }
                    p.trailX[23] = p.x
                    p.trailY[23] = p.y
                }
            }
        }
        
        // Prepare vertex points array to render (line segments)
        var points: [Float] = []
        for p in particles {
            if p.trailCount < 2 { continue }
            let ageFade = getAgeFade(p.age)
            for i in 0..<(p.trailCount - 1) {
                let sx0 = mix(mix(bl.x, br.x, p.trailX[i]), mix(tl.x, tr.x, p.trailX[i]), p.trailY[i])
                let sy0 = mix(mix(bl.y, br.y, p.trailX[i]), mix(tl.y, tr.y, p.trailX[i]), p.trailY[i])
                let ndcX0 = (sx0 / Float(bounds.width)) * 2.0 - 1.0
                let ndcY0 = 1.0 - (sy0 / Float(bounds.height)) * 2.0
                let alpha0 = (Float(i) / 23.0) * ageFade
                
                let sx1 = mix(mix(bl.x, br.x, p.trailX[i+1]), mix(tl.x, tr.x, p.trailX[i+1]), p.trailY[i+1])
                let sy1 = mix(mix(bl.y, br.y, p.trailX[i+1]), mix(tl.y, tr.y, p.trailX[i+1]), p.trailY[i+1])
                let ndcX1 = (sx1 / Float(bounds.width)) * 2.0 - 1.0
                let ndcY1 = 1.0 - (sy1 / Float(bounds.height)) * 2.0
                let alpha1 = (Float(i + 1) / 23.0) * ageFade
                
                points.append(ndcX0)
                points.append(ndcY0)
                points.append(alpha0)
                
                points.append(ndcX1)
                points.append(ndcY1)
                points.append(alpha1)
            }
        }
        
        if points.isEmpty { return }
        
        let bytesLength = points.count * MemoryLayout<Float>.size
        guard let lineBuffer = device?.makeBuffer(bytes: points, length: bytesLength, options: []) else { return }
        
        guard let pState = particlePipelineState else { return }
        renderEncoder.setRenderPipelineState(pState)
        renderEncoder.setVertexBuffer(lineBuffer, offset: 0, index: 0)
        
        var op = opacity
        renderEncoder.setFragmentBytes(&op, length: MemoryLayout<Float>.size, index: 0)
        
        renderEncoder.drawPrimitives(type: .line, vertexStart: 0, vertexCount: points.count / 3)
    }

    
    private func resetParticle(_ p: Particle) {
        p.x = Float.random(in: 0...1)
        p.y = Float.random(in: 0...1)
        p.age = 0
        p.lifetime = Float.random(in: 6.6...13.3)
        p.trailCount = 0
        p.updateCount = 0
    }
    
    private func smoothstep(_ edge0: Float, _ edge1: Float, _ x: Float) -> Float {
        let t = min(max((x - edge0) / (edge1 - edge0), 0.0), 1.0)
        return t * t * (3.0 - 2.0 * t)
    }
    
    private func getAgeFade(_ age: Float) -> Float {
        return smoothstep(0.0, 0.45, age) * smoothstep(1.0, 0.55, age)
    }
    
    private func mix(_ start: Float, _ end: Float, _ fraction: Float) -> Float {
        return start + fraction * (end - start)
    }
}

extension Comparable {
    func clamped(to limits: ClosedRange<Self>) -> Self {
        return min(max(self, limits.lowerBound), limits.upperBound)
    }
}
