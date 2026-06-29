package com.yannicked.nimbus

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.opengl.GLES20
import android.opengl.GLSurfaceView
import android.opengl.GLUtils
import android.util.Log
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.FloatBuffer
import javax.microedition.khronos.egl.EGLConfig
import javax.microedition.khronos.opengles.GL10
import kotlin.math.sqrt
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Callback
import okhttp3.Call
import okhttp3.Response
import java.io.IOException
import java.util.concurrent.TimeUnit

class RadarGLOverlay(private val context: Context) : GLSurfaceView.Renderer {

    var glSurfaceView: GLSurfaceView? = null

    // Projection Sync Lock and variables
    private val projectionLock = Any()
    private var syncBlX = 0f; private var syncBlY = 0f
    private var syncBrX = 0f; private var syncBrY = 0f
    private var syncTlX = 0f; private var syncTlY = 0f
    private var syncTrX = 0f; private var syncTrY = 0f

    fun updateProjection(blX: Float, blY: Float, brX: Float, brY: Float, tlX: Float, tlY: Float, trX: Float, trY: Float) {
        synchronized(projectionLock) {
            syncBlX = blX; syncBlY = blY
            syncBrX = brX; syncBrY = brY
            syncTlX = tlX; syncTlY = tlY
            syncTrX = trX; syncTrY = trY
        }
    }
    @Volatile var opacity = 0.7f
    @Volatile var layerMode = 0f // 0 = rain, 1 = temp, 2 = solar, 3 = wind speed field

    // Texture state
    private var activeTextureId = -1
    private var pendingBitmap: Bitmap? = null
    private var activeBitmap: Bitmap? = null
    private val textureLock = Any()

    private var screenWidth = 1f
    private var screenHeight = 1f

    // Shader Program
    private var programId = -1
    private var particleProgramId = -1

    // Quad buffers
    private lateinit var vertexBuffer: FloatBuffer
    private lateinit var texCoordBuffer: FloatBuffer

    // Colors & Values stops
    private var valStops = FloatArray(8)
    private var colStops = FloatArray(32) // 8 colors * 4 components (RGBA)

    // Wind Particle Simulation
    private val numParticles = 2000
    private val particles = Array(numParticles) { Particle() }
    private var windBitmap: Bitmap? = null
    private var lastFrameTime = System.currentTimeMillis()

    private val httpClient = OkHttpClient.Builder()
        .connectTimeout(15, TimeUnit.SECONDS)
        .readTimeout(15, TimeUnit.SECONDS)
        .dispatcher(okhttp3.Dispatcher().apply {
            maxRequests = 64
            maxRequestsPerHost = 16
        })
        .build()
    @Volatile private var activeRequestSeq = 0
    private val bitmapCache = android.util.LruCache<String, Bitmap>(120)

    class Particle {
        var x = Math.random().toFloat() // 0..1 normalized grid
        var y = Math.random().toFloat() // 0..1 normalized grid
        var age = (Math.random() * 0.8f).toFloat()
        var lifetime = (6.6f + Math.random() * 6.7f).toFloat()
        val trailX = FloatArray(24)
        val trailY = FloatArray(24)
        var trailCount = 0
        var updateCount = 0
    }

    init {
        setupBuffers()
    }

    fun setStops(values: FloatArray, colors: FloatArray) {
        valStops = values
        colStops = colors
    }

    fun cancelStaleRequests(keepUrls: Set<String>) {
        for (call in httpClient.dispatcher.queuedCalls()) {
            if (!keepUrls.contains(call.request().url.toString())) {
                call.cancel()
            }
        }
        for (call in httpClient.dispatcher.runningCalls()) {
            if (!keepUrls.contains(call.request().url.toString())) {
                call.cancel()
            }
        }
    }

    fun loadTextureAsync(urlStr: String, seq: Int, isWind: Boolean = false) {
        if (seq < activeRequestSeq) return
        activeRequestSeq = seq

        // Check Cache first
        val cached = bitmapCache.get(urlStr)
        if (cached != null) {
            synchronized(textureLock) {
                if (seq >= activeRequestSeq) {
                    pendingBitmap = cached
                    if (layerMode == 3f) {
                        windBitmap = cached
                    }
                }
            }
            glSurfaceView?.requestRender()
            return
        }

        val request = Request.Builder().url(urlStr).build()
        httpClient.newCall(request).enqueue(object : Callback {
            override fun onFailure(call: Call, e: IOException) {
                if (call.isCanceled()) return
                Log.e("NimbusGL", "Failed to load image from $urlStr: ${e.message}")
            }

            override fun onResponse(call: Call, response: Response) {
                response.use {
                    if (!response.isSuccessful) {
                        Log.e("NimbusGL", "Failed to load image: HTTP ${response.code}")
                        return
                    }
                    val body = response.body ?: return
                    val options = BitmapFactory.Options().apply { inPremultiplied = false }
                    val bitmap = BitmapFactory.decodeStream(body.byteStream(), null, options)
                    if (bitmap != null) {
                        bitmapCache.put(urlStr, bitmap)
                        var shouldRender = false
                        synchronized(textureLock) {
                            if (seq >= activeRequestSeq) {
                                pendingBitmap = bitmap
                                if (layerMode == 3f) {
                                    windBitmap = bitmap
                                }
                                shouldRender = true
                            }
                        }
                        if (shouldRender) {
                            glSurfaceView?.requestRender()
                        }
                    }
                }
            }
        })
    }

    fun prefetchTexture(urlStr: String, isWind: Boolean = false) {
        if (bitmapCache.get(urlStr) != null) return

        val request = Request.Builder().url(urlStr).build()
        httpClient.newCall(request).enqueue(object : Callback {
            override fun onFailure(call: Call, e: IOException) {}

            override fun onResponse(call: Call, response: Response) {
                response.use {
                    if (response.isSuccessful) {
                        val body = response.body ?: return
                        val options = BitmapFactory.Options().apply { inPremultiplied = false }
                        val bitmap = BitmapFactory.decodeStream(body.byteStream(), null, options)
                        if (bitmap != null) {
                            bitmapCache.put(urlStr, bitmap)
                        }
                    }
                }
            }
        })

    }

    private fun setupBuffers() {
        // Quad vertices: BL, BR, TL, TR (2 triangles)
        val vertices = floatArrayOf(
            0f, 0f, // BL
            1f, 0f, // BR
            0f, 1f, // TL
            0f, 1f, // TL
            1f, 0f, // BR
            1f, 1f  // TR
        )
        val bb = ByteBuffer.allocateDirect(vertices.size * 4)
        bb.order(ByteOrder.nativeOrder())
        vertexBuffer = bb.asFloatBuffer()
        vertexBuffer.put(vertices)
        vertexBuffer.position(0)

        // Texture coordinates mapping
        val texCoords = floatArrayOf(
            0f, 0f,
            1f, 0f,
            0f, 1f,
            0f, 1f,
            1f, 0f,
            1f, 1f
        )
        val tb = ByteBuffer.allocateDirect(texCoords.size * 4)
        tb.order(ByteOrder.nativeOrder())
        texCoordBuffer = tb.asFloatBuffer()
        texCoordBuffer.put(texCoords)
        texCoordBuffer.position(0)
    }

    override fun onSurfaceCreated(gl: GL10?, config: EGLConfig?) {
        GLES20.glClearColor(0f, 0f, 0f, 0f)

        // Compile Vertex Shader
        val vsSource = """
            attribute vec2 a_position;
            varying vec2 v_texcoord;
            uniform vec2 u_bl;
            uniform vec2 u_br;
            uniform vec2 u_tl;
            uniform vec2 u_tr;
            uniform vec2 u_screen_size;

            void main() {
                vec2 screen_pos = mix(
                    mix(u_bl, u_br, a_position.x),
                    mix(u_tl, u_tr, a_position.x),
                    a_position.y
                );
                float ndc_x = (screen_pos.x / u_screen_size.x) * 2.0 - 1.0;
                float ndc_y = 1.0 - (screen_pos.y / u_screen_size.y) * 2.0;
                gl_Position = vec4(ndc_x, ndc_y, 0.0, 1.0);
                v_texcoord = a_position;
            }
        """.trimIndent()

        // Compile Fragment Shader
        val fsSource = """
            precision mediump float;
            varying vec2 v_texcoord;
            uniform sampler2D u_texture;
            uniform float u_opacity;
            uniform float u_layer_mode;
            uniform float u_values[8];
            uniform vec4 u_colors[8];

            vec4 getColor(float val) {
                if (val < u_values[0]) return vec4(0.0);
                if (val <= u_values[1]) {
                    float t = (val - u_values[0]) / (u_values[1] - u_values[0]);
                    return mix(u_colors[0], u_colors[1], t);
                }
                if (val <= u_values[2]) {
                    float t = (val - u_values[1]) / (u_values[2] - u_values[1]);
                    return mix(u_colors[1], u_colors[2], t);
                }
                if (val <= u_values[3]) {
                    float t = (val - u_values[2]) / (u_values[3] - u_values[2]);
                    return mix(u_colors[2], u_colors[3], t);
                }
                if (val <= u_values[4]) {
                    float t = (val - u_values[3]) / (u_values[4] - u_values[3]);
                    return mix(u_colors[3], u_colors[4], t);
                }
                if (val <= u_values[5]) {
                    float t = (val - u_values[4]) / (u_values[5] - u_values[4]);
                    return mix(u_colors[4], u_colors[5], t);
                }
                if (val <= u_values[6]) {
                    float t = (val - u_values[5]) / (u_values[6] - u_values[5]);
                    return mix(u_colors[5], u_colors[6], t);
                }
                if (val <= u_values[7]) {
                    float t = (val - u_values[6]) / (u_values[7] - u_values[6]);
                    return mix(u_colors[6], u_colors[7], t);
                }
                return u_colors[7];
            }

            void main() {
                vec2 clamped_coord = vec2(
                    0.5 / 700.0 + v_texcoord.x * (699.0 / 700.0),
                    0.5 / 765.0 + (1.0 - v_texcoord.y) * (764.0 / 765.0)
                );
                vec4 tex = texture2D(u_texture, clamped_coord);
                if (u_layer_mode != 3.0 && tex.a < 0.99) {
                    discard;
                }
                float r = tex.r * 255.0;
                float g = tex.g * 255.0;
                float raw_val = r * 256.0 + g;
                if (raw_val >= 65535.0 || raw_val == 0.0) {
                    discard;
                }
                float val;
                int modeInt = int(u_layer_mode);
                if (modeInt == 1) { // temp
                    val = raw_val / 10.0 - 273.15;
                } else if (modeInt == 2) { // solar
                    val = raw_val;
                } else if (modeInt == 3) { // wind speed field
                    float u_raw = raw_val;
                    float v_raw = tex.b * 255.0 * 256.0 + tex.a * 255.0;
                    float u_phys = u_raw / 100.0 - 100.0;
                    float v_phys = v_raw / 100.0 - 100.0;
                    val = sqrt(u_phys * u_phys + v_phys * v_phys);
                } else { // rain
                    val = raw_val * 0.01;
                }
                vec4 c = getColor(val);
                if (c.a == 0.0) {
                    discard;
                }
                gl_FragColor = vec4(c.rgb, c.a * u_opacity);
            }
        """.trimIndent()

        programId = createProgram(vsSource, fsSource)

        // Compile Vertex Shader for particles
        val pVs = """
            attribute vec2 a_pos;
            attribute float a_alpha;
            varying float v_alpha;
            void main() {
                gl_Position = vec4(a_pos, 0.0, 1.0);
                v_alpha = a_alpha;
            }
        """.trimIndent()

        // Compile Fragment Shader for particles
        val pFs = """
            precision mediump float;
            uniform float u_op;
            varying float v_alpha;
            void main() {
                gl_FragColor = vec4(1.0, 1.0, 1.0, u_op * v_alpha * 0.6);
            }
        """.trimIndent()

        particleProgramId = createProgram(pVs, pFs)
    }

    override fun onSurfaceChanged(gl: GL10?, width: Int, height: Int) {
        GLES20.glViewport(0, 0, width, height)
        screenWidth = width.toFloat()
        screenHeight = height.toFloat()
    }

    override fun onDrawFrame(gl: GL10?) {
        GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT or GLES20.GL_DEPTH_BUFFER_BIT)

        // Check if there is a newly loaded texture bitmap to upload to GPU
        var bitmapToUpload: Bitmap? = null
        synchronized(textureLock) {
            if (pendingBitmap != null) {
                bitmapToUpload = pendingBitmap
                pendingBitmap = null
            }
        }

        if (bitmapToUpload != null) {
            uploadTexture(bitmapToUpload!!)
        }

        if (activeTextureId == -1 || programId == -1) return

        GLES20.glUseProgram(programId)

        // Bind attributes
        val posAttr = GLES20.glGetAttribLocation(programId, "a_position")
        GLES20.glEnableVertexAttribArray(posAttr)
        GLES20.glVertexAttribPointer(posAttr, 2, GLES20.GL_FLOAT, false, 0, vertexBuffer)

        val currentBlX: Float
        val currentBlY: Float
        val currentBrX: Float
        val currentBrY: Float
        val currentTlX: Float
        val currentTlY: Float
        val currentTrX: Float
        val currentTrY: Float
        synchronized(projectionLock) {
            currentBlX = syncBlX; currentBlY = syncBlY
            currentBrX = syncBrX; currentBrY = syncBrY
            currentTlX = syncTlX; currentTlY = syncTlY
            currentTrX = syncTrX; currentTrY = syncTrY
        }

        // Bind uniforms
        GLES20.glUniform2f(GLES20.glGetUniformLocation(programId, "u_bl"), currentBlX, currentBlY)
        GLES20.glUniform2f(GLES20.glGetUniformLocation(programId, "u_br"), currentBrX, currentBrY)
        GLES20.glUniform2f(GLES20.glGetUniformLocation(programId, "u_tl"), currentTlX, currentTlY)
        GLES20.glUniform2f(GLES20.glGetUniformLocation(programId, "u_tr"), currentTrX, currentTrY)
        GLES20.glUniform2f(GLES20.glGetUniformLocation(programId, "u_screen_size"), screenWidth, screenHeight)
        GLES20.glUniform1f(GLES20.glGetUniformLocation(programId, "u_opacity"), opacity)
        GLES20.glUniform1f(GLES20.glGetUniformLocation(programId, "u_layer_mode"), layerMode)

        // Load value stops and color stops arrays
        GLES20.glUniform1fv(GLES20.glGetUniformLocation(programId, "u_values"), 8, valStops, 0)
        GLES20.glUniform4fv(GLES20.glGetUniformLocation(programId, "u_colors"), 8, colStops, 0)

        // Bind texture sampler
        GLES20.glActiveTexture(GLES20.GL_TEXTURE0)
        GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, activeTextureId)
        GLES20.glUniform1i(GLES20.glGetUniformLocation(programId, "u_texture"), 0)

        // Alpha Blending
        GLES20.glEnable(GLES20.GL_BLEND)
        GLES20.glBlendFunc(GLES20.GL_SRC_ALPHA, GLES20.GL_ONE_MINUS_SRC_ALPHA)

        // Draw quad
        GLES20.glDrawArrays(GLES20.GL_TRIANGLES, 0, 6)

        // Wind Particle updates & draw overlay
        if (layerMode == 3f) {
            updateAndDrawParticles(currentBlX, currentBlY, currentBrX, currentBrY, currentTlX, currentTlY, currentTrX, currentTrY)
        }

        // Cleanup
        GLES20.glDisableVertexAttribArray(posAttr)
        GLES20.glDisable(GLES20.GL_BLEND)
    }

    private fun uploadTexture(bitmap: Bitmap) {
        if (activeTextureId != -1) {
            GLES20.glDeleteTextures(1, intArrayOf(activeTextureId), 0)
        }

        val textures = IntArray(1)
        GLES20.glGenTextures(1, textures, 0)
        activeTextureId = textures[0]

        GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, activeTextureId)
        GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MIN_FILTER, GLES20.GL_NEAREST)
        GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MAG_FILTER, GLES20.GL_NEAREST)
        GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_S, GLES20.GL_CLAMP_TO_EDGE)
        GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_T, GLES20.GL_CLAMP_TO_EDGE)

        GLUtils.texImage2D(GLES20.GL_TEXTURE_2D, 0, bitmap, 0)
        GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, 0)

        activeBitmap = bitmap
    }

    private fun getWindVelocityAtPixel(wind: Bitmap, x: Int, y: Int): Pair<Float, Float> {
        val color = wind.getPixel(x, y)
        val r = (color shr 16) and 0xFF
        val g = (color shr 8) and 0xFF
        val b = (color) and 0xFF
        val a = (color shr 24) and 0xFF

        val uRaw = (r * 256 + g).toFloat()
        val vRaw = (b * 256 + a).toFloat()

        if (uRaw >= 65535f || vRaw >= 65535f || uRaw == 0f || vRaw == 0f) {
            return Pair(0f, 0f)
        }

        val u = uRaw / 100.0f - 100.0f
        val v = vRaw / 100.0f - 100.0f
        return Pair(u, v)
    }

    private fun getWindVelocityInterpolated(wind: Bitmap, xNorm: Float, yNorm: Float): Pair<Float, Float> {
        val wW = wind.width
        val wH = wind.height
        if (wW <= 1 || wH <= 1) return Pair(0f, 0f)

        val px = xNorm * (wW - 1)
        val py = (1f - yNorm) * (wH - 1)

        val x0 = px.toInt().coerceIn(0, wW - 1)
        val y0 = py.toInt().coerceIn(0, wH - 1)
        val x1 = (x0 + 1).coerceIn(0, wW - 1)
        val y1 = (y0 + 1).coerceIn(0, wH - 1)

        val tx = px - x0
        val ty = py - y0

        val p00 = getWindVelocityAtPixel(wind, x0, y0)
        val p10 = getWindVelocityAtPixel(wind, x1, y0)
        val p01 = getWindVelocityAtPixel(wind, x0, y1)
        val p11 = getWindVelocityAtPixel(wind, x1, y1)

        val u0 = p00.first + tx * (p10.first - p00.first)
        val u1 = p01.first + tx * (p11.first - p01.first)
        val u = u0 + ty * (u1 - u0)

        val v0 = p00.second + tx * (p10.second - p00.second)
        val v1 = p01.second + tx * (p11.second - p01.second)
        val v = v0 + ty * (v1 - v0)

        return Pair(u, v)
    }

    private fun updateAndDrawParticles(
        blX: Float, blY: Float,
        brX: Float, brY: Float,
        tlX: Float, tlY: Float,
        trX: Float, trY: Float
    ) {
        val wind: Bitmap
        synchronized(textureLock) {
            wind = windBitmap ?: return
        }
        val now = System.currentTimeMillis()
        var dt = (now - lastFrameTime) / 1000f
        if (dt > 0.1f) dt = 0.1f
        lastFrameTime = now

        val wW = wind.width
        val wH = wind.height
        if (wW == 0 || wH == 0) return

        // 1. Update particle coordinates on CPU
        for (p in particles) {
            p.age += dt / p.lifetime
            if (p.age >= 1.0f || p.x < 0f || p.x > 1f || p.y < 0f || p.y > 1f) {
                resetParticle(p)
                continue
            }

            // Read wind pixel color at particle coordinate with bilinear interpolation
            val (u, v) = getWindVelocityInterpolated(wind, p.x, p.y)
            if (u == 0f && v == 0f) {
                resetParticle(p)
                continue
            }

            // Update particle position (Mercator delta scaling)
            val speedFactor = 2.5f * 4.0f
            val dxNorm = (u * dt * speedFactor * 1200f) / 1210000f
            val dyNorm = (v * dt * speedFactor * 1200f) / 1310000f

            p.x += dxNorm
            p.y += dyNorm

            // Shift trails every 2 frames to achieve 48 frames of history (matching WebGL)
            p.updateCount++
            if (p.updateCount % 2 == 0) {
                if (p.trailCount < 24) {
                    p.trailX[p.trailCount] = p.x
                    p.trailY[p.trailCount] = p.y
                    p.trailCount++
                } else {
                    for (i in 0 until 23) {
                        p.trailX[i] = p.trailX[i + 1]
                        p.trailY[i] = p.trailY[i + 1]
                    }
                    p.trailX[23] = p.x
                    p.trailY[23] = p.y
                }
            }
        }

        // 2. Draw particle lines on the Canvas overlay via OpenGL
        // Prepare vertex points array: each line segment has 2 vertices, each vertex has 3 floats (x, y, alpha)
        val particleVertices = FloatArray(numParticles * 23 * 2 * 3)
        var count = 0

        for (p in particles) {
            if (p.trailCount < 2) continue
            val ageFade = getAgeFade(p.age)
            for (i in 0 until (p.trailCount - 1)) {
                // Point A
                val sx0 = mix(mix(blX, brX, p.trailX[i]), mix(tlX, trX, p.trailX[i]), p.trailY[i])
                val sy0 = mix(mix(blY, brY, p.trailX[i]), mix(tlY, trY, p.trailX[i]), p.trailY[i])
                val ndcX0 = (sx0 / screenWidth) * 2f - 1f
                val ndcY0 = 1f - (sy0 / screenHeight) * 2f
                val alpha0 = (i.toFloat() / 23f) * ageFade

                // Point B
                val sx1 = mix(mix(blX, brX, p.trailX[i+1]), mix(tlX, trX, p.trailX[i+1]), p.trailY[i+1])
                val sy1 = mix(mix(blY, brY, p.trailX[i+1]), mix(tlY, trY, p.trailX[i+1]), p.trailY[i+1])
                val ndcX1 = (sx1 / screenWidth) * 2f - 1f
                val ndcY1 = 1f - (sy1 / screenHeight) * 2f
                val alpha1 = ((i + 1).toFloat() / 23f) * ageFade

                particleVertices[count++] = ndcX0
                particleVertices[count++] = ndcY0
                particleVertices[count++] = alpha0

                particleVertices[count++] = ndcX1
                particleVertices[count++] = ndcY1
                particleVertices[count++] = alpha1
            }
        }

        if (count == 0) return

        val pBuffer = ByteBuffer.allocateDirect(count * 4).order(ByteOrder.nativeOrder()).asFloatBuffer()
        pBuffer.put(particleVertices, 0, count).position(0)

        // Draw lines
        GLES20.glEnable(GLES20.GL_BLEND)
        GLES20.glBlendFunc(GLES20.GL_SRC_ALPHA, GLES20.GL_ONE)

        val pProg = particleProgramId
        if (pProg != -1) {
            GLES20.glUseProgram(pProg)

            val posA = GLES20.glGetAttribLocation(pProg, "a_pos")
            GLES20.glEnableVertexAttribArray(posA)
            pBuffer.position(0)
            GLES20.glVertexAttribPointer(posA, 2, GLES20.GL_FLOAT, false, 12, pBuffer)

            val alphaA = GLES20.glGetAttribLocation(pProg, "a_alpha")
            GLES20.glEnableVertexAttribArray(alphaA)
            pBuffer.position(2)
            GLES20.glVertexAttribPointer(alphaA, 1, GLES20.GL_FLOAT, false, 12, pBuffer)

            GLES20.glUniform1f(GLES20.glGetUniformLocation(pProg, "u_op"), opacity)

            GLES20.glLineWidth(2.5f)
            GLES20.glDrawArrays(GLES20.GL_LINES, 0, count / 3)
            GLES20.glLineWidth(1.0f)

            GLES20.glDisableVertexAttribArray(posA)
            GLES20.glDisableVertexAttribArray(alphaA)
        }
    }

    private fun resetParticle(p: Particle) {
        p.x = Math.random().toFloat()
        p.y = Math.random().toFloat()
        p.age = 0f
        p.lifetime = (6.6f + Math.random() * 6.7f).toFloat()
        p.trailCount = 0
        p.updateCount = 0
    }


    private fun smoothstep(edge0: Float, edge1: Float, x: Float): Float {
        val t = ((x - edge0) / (edge1 - edge0)).coerceIn(0f, 1f)
        return t * t * (3f - 2f * t)
    }

    private fun getAgeFade(age: Float): Float {
        return smoothstep(0.0f, 0.45f, age) * smoothstep(1.0f, 0.55f, age)
    }

    private fun mix(start: Float, end: Float, fraction: Float): Float {
        return start + fraction * (end - start)
    }

    private fun createProgram(vsCode: String, fsCode: String): Int {
        val vs = loadShader(GLES20.GL_VERTEX_SHADER, vsCode)
        val fs = loadShader(GLES20.GL_FRAGMENT_SHADER, fsCode)
        if (vs == 0 || fs == 0) return -1

        val prog = GLES20.glCreateProgram()
        GLES20.glAttachShader(prog, vs)
        GLES20.glAttachShader(prog, fs)
        GLES20.glLinkProgram(prog)

        val linkStatus = IntArray(1)
        GLES20.glGetProgramiv(prog, GLES20.GL_LINK_STATUS, linkStatus, 0)
        if (linkStatus[0] == 0) {
            Log.e("NimbusGL", "Link error: " + GLES20.glGetProgramInfoLog(prog))
            GLES20.glDeleteProgram(prog)
            return -1
        }
        return prog
    }

    private fun loadShader(type: Int, shaderCode: String): Int {
        val shader = GLES20.glCreateShader(type)
        GLES20.glShaderSource(shader, shaderCode)
        GLES20.glCompileShader(shader)

        val compiled = IntArray(1)
        GLES20.glGetShaderiv(shader, GLES20.GL_COMPILE_STATUS, compiled, 0)
        if (compiled[0] == 0) {
            Log.e("NimbusGL", "Compile error ($type): " + GLES20.glGetShaderInfoLog(shader))
            GLES20.glDeleteShader(shader)
            return 0
        }
        return shader
    }
}
