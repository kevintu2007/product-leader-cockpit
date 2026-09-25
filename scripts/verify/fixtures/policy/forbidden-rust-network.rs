fn connect() {
    let _stream = std::net::TcpStream::connect("127.0.0.1:9");
    let _socket = tokio::net::UdpSocket::bind("127.0.0.1:0");
}
