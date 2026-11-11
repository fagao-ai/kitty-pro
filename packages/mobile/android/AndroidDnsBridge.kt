package com.kitty.pro

import android.content.Context
import android.net.ConnectivityManager
import android.net.DnsResolver
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Build
import android.os.CancellationSignal
import android.util.Log
import java.io.ByteArrayOutputStream
import java.io.DataOutputStream
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.Inet4Address
import java.net.Inet6Address
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.SocketAddress
import java.net.SocketException
import java.net.UnknownHostException
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.ThreadFactory
import java.util.concurrent.atomic.AtomicInteger

/**
 * Adapts Android's network-aware DNS resolver to a loopback UDP endpoint that
 * the c-shared sing-box core can consume without embedding a JVM object in Go.
 */
object AndroidDnsBridge {
    private const val TAG = "KittyDnsBridge"
    private const val DNS_PORT = 53
    private const val DNS_HEADER_SIZE = 12
    private const val TYPE_A = 1
    private const val TYPE_AAAA = 28
    private const val CLASS_IN = 1
    private const val RCODE_NO_ERROR = 0
    private const val RCODE_NAME_ERROR = 3
    private const val RCODE_SERVER_FAILURE = 2
    private const val LEGACY_DNS_TIMEOUT_MS = 3_000

    private val threadIds = AtomicInteger()
    private val resolverExecutor: ExecutorService = Executors.newFixedThreadPool(
        4,
        daemonThreadFactory("kitty-dns-worker"),
    )
    private val candidates = ConcurrentHashMap<Network, NetworkCapabilities>()

    @Volatile
    private var connectivity: ConnectivityManager? = null

    @Volatile
    private var selectedNetwork: Network? = null

    @Volatile
    private var localSocket: DatagramSocket? = null

    @Volatile
    private var startupError = ""

    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            val capabilities = connectivity?.getNetworkCapabilities(network) ?: return
            updateCandidate(network, capabilities)
        }

        override fun onCapabilitiesChanged(
            network: Network,
            networkCapabilities: NetworkCapabilities,
        ) {
            updateCandidate(network, networkCapabilities)
        }

        override fun onLost(network: Network) {
            candidates.remove(network)
            refreshSelectedNetwork()
        }
    }

    @JvmStatic
    fun localDnsPort(context: Context): Int {
        localSocket?.let { return it.localPort }
        synchronized(this) {
            localSocket?.let { return it.localPort }
            return try {
                start(context.applicationContext)
                startupError = ""
                localSocket?.localPort ?: -1
            } catch (error: Throwable) {
                startupError = error.message ?: error.javaClass.simpleName
                Log.e(TAG, "Unable to start Android DNS adapter", error)
                -1
            }
        }
    }

    @JvmStatic
    fun lastError(): String = startupError

    private fun start(context: Context) {
        val manager = context.getSystemService(ConnectivityManager::class.java)
            ?: error("Android connectivity service is unavailable")
        connectivity = manager

        val request = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
            .build()
        manager.registerNetworkCallback(request, networkCallback)
        manager.allNetworks.forEach { network ->
            manager.getNetworkCapabilities(network)?.let { updateCandidate(network, it) }
        }
        refreshSelectedNetwork()

        val socket = DatagramSocket(null).apply {
            reuseAddress = false
            bind(InetSocketAddress(InetAddress.getByName("127.0.0.1"), 0))
        }
        localSocket = socket
        daemonThreadFactory("kitty-dns-listener").newThread {
            receiveQueries(socket)
        }.start()
        Log.i(TAG, "Android DNS adapter listening on 127.0.0.1:${socket.localPort}")
    }

    private fun receiveQueries(socket: DatagramSocket) {
        val receiveBuffer = ByteArray(65_535)
        while (!socket.isClosed) {
            try {
                val packet = DatagramPacket(receiveBuffer, receiveBuffer.size)
                socket.receive(packet)
                val query = packet.data.copyOfRange(packet.offset, packet.offset + packet.length)
                val client = packet.socketAddress
                resolve(query, client)
            } catch (_: SocketException) {
                return
            } catch (error: Throwable) {
                Log.w(TAG, "Unable to receive local DNS query", error)
            }
        }
    }

    private fun resolve(query: ByteArray, client: SocketAddress) {
        val network = selectedNetwork ?: refreshSelectedNetwork()
        if (network == null) {
            sendResponse(serverFailure(query), client)
            return
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            resolveRaw(network, query, client)
        } else {
            resolverExecutor.execute { resolveLegacy(network, query, client) }
        }
    }

    private fun resolveRaw(network: Network, query: ByteArray, client: SocketAddress) {
        try {
            DnsResolver.getInstance().rawQuery(
                network,
                query,
                DnsResolver.FLAG_NO_RETRY,
                resolverExecutor,
                CancellationSignal(),
                object : DnsResolver.Callback<ByteArray> {
                    override fun onAnswer(answer: ByteArray, rcode: Int) {
                        // rawQuery returns a complete packet for DNS rcodes such as NXDOMAIN.
                        val response = if (answer.size >= DNS_HEADER_SIZE) answer else serverFailure(query)
                        sendResponse(response, client)
                    }

                    override fun onError(error: DnsResolver.DnsException) {
                        sendResponse(serverFailure(query), client)
                    }
                },
            )
        } catch (_: Throwable) {
            sendResponse(serverFailure(query), client)
        }
    }

    private fun resolveLegacy(network: Network, query: ByteArray, client: SocketAddress) {
        val question = parseQuestion(query)
        if (question != null && (question.type == TYPE_A || question.type == TYPE_AAAA)) {
            try {
                val addresses = network.getAllByName(question.name).filter { address ->
                    (question.type == TYPE_A && address is Inet4Address) ||
                        (question.type == TYPE_AAAA && address is Inet6Address)
                }
                sendResponse(addressResponse(query, question, addresses), client)
            } catch (_: UnknownHostException) {
                sendResponse(errorResponse(query, question.endOffset, RCODE_NAME_ERROR), client)
            } catch (_: Throwable) {
                sendResponse(serverFailure(query), client)
            }
            return
        }

        val manager = connectivity
        val dnsServers = manager?.getLinkProperties(network)?.dnsServers.orEmpty()
        for (server in dnsServers) {
            try {
                DatagramSocket(null).use { upstream ->
                    network.bindSocket(upstream)
                    upstream.soTimeout = LEGACY_DNS_TIMEOUT_MS
                    upstream.connect(InetSocketAddress(server, DNS_PORT))
                    upstream.send(DatagramPacket(query, query.size))
                    val buffer = ByteArray(65_535)
                    val packet = DatagramPacket(buffer, buffer.size)
                    upstream.receive(packet)
                    val response = packet.data.copyOfRange(packet.offset, packet.offset + packet.length)
                    if (sameTransaction(query, response)) {
                        sendResponse(response, client)
                        return
                    }
                }
            } catch (_: Throwable) {
                // Try the next DNS server supplied by this physical network.
            }
        }
        sendResponse(serverFailure(query), client)
    }

    private fun sendResponse(response: ByteArray, client: SocketAddress) {
        if (response.isEmpty()) return
        val socket = localSocket ?: return
        try {
            socket.send(DatagramPacket(response, response.size, client))
        } catch (_: Throwable) {
            // The sing-box query may have timed out or the process may be stopping.
        }
    }

    private fun updateCandidate(network: Network, capabilities: NetworkCapabilities) {
        if (
            capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) &&
            capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
        ) {
            candidates[network] = capabilities
        } else {
            candidates.remove(network)
        }
        refreshSelectedNetwork()
    }

    private fun refreshSelectedNetwork(): Network? {
        val manager = connectivity ?: return null
        val active = runCatching { manager.activeNetwork }.getOrNull()
        val next = candidates.entries.maxByOrNull { (network, capabilities) ->
            networkScore(network, capabilities, active)
        }?.key
        if (selectedNetwork != next) {
            selectedNetwork = next
            Log.i(TAG, "Android DNS physical network changed: ${next ?: "none"}")
        }
        return next
    }

    private fun networkScore(
        network: Network,
        capabilities: NetworkCapabilities,
        active: Network?,
    ): Int {
        var score = if (network == active) 10_000 else 0
        if (capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)) score += 1_000
        score += when {
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) -> 40
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> 30
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> 20
            else -> 10
        }
        return score
    }

    private data class DnsQuestion(
        val name: String,
        val type: Int,
        val endOffset: Int,
    )

    private fun parseQuestion(query: ByteArray): DnsQuestion? {
        if (query.size < DNS_HEADER_SIZE || unsignedShort(query, 4) != 1) return null
        var offset = DNS_HEADER_SIZE
        val labels = mutableListOf<String>()
        while (offset < query.size) {
            val length = query[offset].toInt() and 0xff
            offset += 1
            if (length == 0) break
            // Compression pointers are not expected in a one-question DNS request.
            if (length > 63 || offset + length > query.size) return null
            labels += query.copyOfRange(offset, offset + length).toString(Charsets.US_ASCII)
            offset += length
        }
        if (labels.isEmpty() || offset + 4 > query.size) return null
        val type = unsignedShort(query, offset)
        val queryClass = unsignedShort(query, offset + 2)
        if (queryClass != CLASS_IN) return null
        return DnsQuestion(labels.joinToString("."), type, offset + 4)
    }

    private fun addressResponse(
        query: ByteArray,
        question: DnsQuestion,
        addresses: List<InetAddress>,
    ): ByteArray {
        val output = ByteArrayOutputStream()
        DataOutputStream(output).use { data ->
            writeHeader(data, query, RCODE_NO_ERROR, addresses.size)
            data.write(query, DNS_HEADER_SIZE, question.endOffset - DNS_HEADER_SIZE)
            addresses.forEach { address ->
                data.writeShort(0xc00c)
                data.writeShort(question.type)
                data.writeShort(CLASS_IN)
                data.writeInt(60)
                data.writeShort(address.address.size)
                data.write(address.address)
            }
        }
        return output.toByteArray()
    }

    private fun errorResponse(query: ByteArray, questionEnd: Int, rcode: Int): ByteArray {
        val output = ByteArrayOutputStream()
        DataOutputStream(output).use { data ->
            writeHeader(data, query, rcode, 0)
            data.write(query, DNS_HEADER_SIZE, questionEnd - DNS_HEADER_SIZE)
        }
        return output.toByteArray()
    }

    private fun serverFailure(query: ByteArray): ByteArray {
        if (query.size < DNS_HEADER_SIZE) return ByteArray(0)
        val question = parseQuestion(query)
        return if (question != null) {
            errorResponse(query, question.endOffset, RCODE_SERVER_FAILURE)
        } else {
            ByteArray(DNS_HEADER_SIZE).also { response ->
                response[0] = query[0]
                response[1] = query[1]
                response[2] = ((query[2].toInt() and 0x79) or 0x80).toByte()
                response[3] = (0x80 or RCODE_SERVER_FAILURE).toByte()
            }
        }
    }

    private fun writeHeader(
        output: DataOutputStream,
        query: ByteArray,
        rcode: Int,
        answerCount: Int,
    ) {
        output.writeByte(query[0].toInt())
        output.writeByte(query[1].toInt())
        output.writeByte((query[2].toInt() and 0x79) or 0x80)
        output.writeByte(0x80 or rcode)
        output.writeShort(1)
        output.writeShort(answerCount)
        output.writeShort(0)
        output.writeShort(0)
    }

    private fun sameTransaction(query: ByteArray, response: ByteArray): Boolean =
        query.size >= 2 && response.size >= 2 && query[0] == response[0] && query[1] == response[1]

    private fun unsignedShort(bytes: ByteArray, offset: Int): Int =
        ((bytes[offset].toInt() and 0xff) shl 8) or (bytes[offset + 1].toInt() and 0xff)

    private fun daemonThreadFactory(prefix: String): ThreadFactory = ThreadFactory { task ->
        Thread(task, "$prefix-${threadIds.incrementAndGet()}").apply { isDaemon = true }
    }
}
