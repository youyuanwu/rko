//! Userspace test for the rko_http io_uring kernel module.
//!
//! Tests the `/dev/rko_http` misc device by sending io_uring custom
//! commands (IORING_OP_URING_CMD) and verifying the round-trip:
//!
//! 1. SERVER_START — start HTTP server on port 8080
//! 2. CREATE_QUEUE — create a request queue
//! 3. ADD_URL — register "/*" catch-all route
//! 4. Send an HTTP request via TCP to port 8080
//! 5. RECV_REQUEST — receive the parsed request from the kernel
//! 6. SEND_RESPONSE — send a response back through the kernel
//! 7. Verify the TCP client got the correct HTTP response

use io_uring::{cqueue, opcode, squeue, types, IoUring};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::fd::AsRawFd;
use std::{fs, thread, time};

// Command opcodes (must match rko_util::http::uring constants)
const HTTP_CMD_SERVER_START: u32 = 0;
const HTTP_CMD_SERVER_STOP: u32 = 1;
const HTTP_CMD_CREATE_QUEUE: u32 = 2;
const HTTP_CMD_ADD_URL: u32 = 4;
const HTTP_CMD_RECV_REQUEST: u32 = 6;
const HTTP_CMD_SEND_RESPONSE: u32 = 7;

// Request header layout (must match rko_util::http::uring::RequestHdr)
#[repr(C)]
struct RequestHdr {
    req_id: u64,
    method: u8,
    version: u8,
    header_count: u16,
    path_len: u32,
    body_len: u32,
    total_len: u32,
}

fn submit_cmd(
    ring: &mut IoUring<squeue::Entry128, cqueue::Entry>,
    fd: i32,
    cmd_op: u32,
    cmd_data: &[u8],
) -> i32 {
    let mut cmd = [0u8; 80];
    let len = cmd_data.len().min(80);
    cmd[..len].copy_from_slice(&cmd_data[..len]);

    let entry = opcode::UringCmd80::new(types::Fd(fd), cmd_op)
        .cmd(cmd)
        .build();

    unsafe { ring.submission().push(&entry).expect("SQ full") };
    ring.submit_and_wait(1).expect("submit failed");

    let cqe = ring.completion().next().expect("no CQE");
    cqe.result()
}

fn main() {
    println!("http_uring_test: opening /dev/rko_http");
    let dev = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/rko_http")
        .expect("failed to open /dev/rko_http");

    let mut ring: IoUring<squeue::Entry128, cqueue::Entry> =
        IoUring::builder().build(32).expect("io_uring init failed");

    let fd = dev.as_raw_fd();

    // 1. CREATE_QUEUE
    println!("http_uring_test: creating queue");
    let mut cmd_data = [0u8; 8];
    cmd_data[0..4].copy_from_slice(&256u32.to_ne_bytes());
    let queue_id = submit_cmd(&mut ring, fd, HTTP_CMD_CREATE_QUEUE, &cmd_data);
    assert!(queue_id > 0, "CREATE_QUEUE failed: {queue_id}");
    println!("http_uring_test: queue_id={queue_id}");

    // 2. ADD_URL "/*"
    println!("http_uring_test: adding URL /*");
    let url = b"/*";
    let mut cmd_data = [0u8; 24];
    cmd_data[0..4].copy_from_slice(&(queue_id as u32).to_ne_bytes());
    cmd_data[4..8].copy_from_slice(&(url.len() as u32).to_ne_bytes());
    cmd_data[8..16].copy_from_slice(&(url.as_ptr() as u64).to_ne_bytes());
    let ret = submit_cmd(&mut ring, fd, HTTP_CMD_ADD_URL, &cmd_data);
    assert!(ret == 0, "ADD_URL failed: {ret}");
    println!("http_uring_test: URL added");

    // 3. RECV_REQUEST before server start (expect EAGAIN)
    println!("http_uring_test: recv_request (expect EAGAIN)");
    let mut recv_buf = vec![0u8; 4096];
    let mut cmd_data = [0u8; 16];
    cmd_data[0..4].copy_from_slice(&(queue_id as u32).to_ne_bytes());
    cmd_data[4..8].copy_from_slice(&(recv_buf.len() as u32).to_ne_bytes());
    cmd_data[8..16].copy_from_slice(&(recv_buf.as_mut_ptr() as u64).to_ne_bytes());
    let ret = submit_cmd(&mut ring, fd, HTTP_CMD_RECV_REQUEST, &cmd_data);
    assert!(ret == -11, "RECV_REQUEST expected EAGAIN (-11), got {ret}");
    println!("http_uring_test: got EAGAIN as expected");

    // 4. SERVER_START on :8080
    println!("http_uring_test: starting server on :8080");
    let mut cmd_data = [0u8; 8];
    cmd_data[0..4].copy_from_slice(&0u32.to_ne_bytes());
    cmd_data[4..6].copy_from_slice(&8080u16.to_be_bytes());
    let ret = submit_cmd(&mut ring, fd, HTTP_CMD_SERVER_START, &cmd_data);
    assert!(ret == 0, "SERVER_START failed: {ret}");
    println!("http_uring_test: server started");

    // Give server time to start accepting
    thread::sleep(time::Duration::from_millis(100));

    // 5. Send HTTP request via TCP
    println!("http_uring_test: sending HTTP request");
    let mut tcp = TcpStream::connect("127.0.0.1:8080").expect("TCP connect failed");
    tcp.write_all(b"GET /hello HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .expect("TCP write failed");

    // Give kernel time to parse and enqueue
    thread::sleep(time::Duration::from_millis(200));

    // 6. RECV_REQUEST (should have the parsed request)
    println!("http_uring_test: receiving request via io_uring");
    let mut recv_buf = vec![0u8; 4096];
    let mut cmd_data = [0u8; 16];
    cmd_data[0..4].copy_from_slice(&(queue_id as u32).to_ne_bytes());
    cmd_data[4..8].copy_from_slice(&(recv_buf.len() as u32).to_ne_bytes());
    cmd_data[8..16].copy_from_slice(&(recv_buf.as_mut_ptr() as u64).to_ne_bytes());
    let ret = submit_cmd(&mut ring, fd, HTTP_CMD_RECV_REQUEST, &cmd_data);
    assert!(ret > 0, "RECV_REQUEST failed: {ret}");
    println!("http_uring_test: received {ret} bytes");

    let hdr = unsafe { &*(recv_buf.as_ptr() as *const RequestHdr) };
    assert!(hdr.req_id > 0, "req_id is 0");
    assert!(hdr.method == 0, "expected GET (0), got {}", hdr.method);
    assert!(hdr.path_len > 0, "path_len is 0");
    println!(
        "http_uring_test: req_id={} method=GET path_len={}",
        hdr.req_id, hdr.path_len
    );

    let path_start = std::mem::size_of::<RequestHdr>();
    let path = &recv_buf[path_start..path_start + hdr.path_len as usize];
    let path_str = std::str::from_utf8(path).expect("invalid path UTF-8");
    assert!(path_str == "/hello", "expected /hello, got {path_str}");
    println!("http_uring_test: path={path_str}");

    // 7. SEND_RESPONSE
    println!("http_uring_test: sending response");
    let body = b"Hello from userspace!\n";
    let mut cmd_data = [0u8; 24];
    cmd_data[0..8].copy_from_slice(&hdr.req_id.to_ne_bytes());
    cmd_data[8..10].copy_from_slice(&200u16.to_ne_bytes());
    cmd_data[10..12].copy_from_slice(&0u16.to_ne_bytes());
    cmd_data[12..16].copy_from_slice(&(body.len() as u32).to_ne_bytes());
    cmd_data[16..24].copy_from_slice(&(body.as_ptr() as u64).to_ne_bytes());
    let ret = submit_cmd(&mut ring, fd, HTTP_CMD_SEND_RESPONSE, &cmd_data);
    assert!(ret == 0, "SEND_RESPONSE failed: {ret}");
    println!("http_uring_test: response sent");

    // 8. Verify TCP response
    let mut response = Vec::new();
    tcp.read_to_end(&mut response).expect("TCP read failed");
    let response_str = String::from_utf8_lossy(&response);
    assert!(
        response_str.contains("200 OK"),
        "expected HTTP 200, got: {response_str}"
    );
    assert!(
        response_str.contains("Hello from userspace!"),
        "expected body in response, got: {response_str}"
    );
    println!("http_uring_test: verified HTTP 200 with correct body");

    // 9. Cleanup
    let ret = submit_cmd(&mut ring, fd, HTTP_CMD_SERVER_STOP, &[]);
    assert!(ret == 0, "SERVER_STOP failed: {ret}");
    println!("http_uring_test: server stopped");

    println!("http_uring_test: ALL TESTS PASSED");
}
