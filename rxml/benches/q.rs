use std::io;
use std::io::Read;

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use bytes::{Buf, Bytes};

use rxml::BufferQueue;

/* fn buffer_queue(c: &mut Criterion) {
	let mut group = c.benchmark_group("single-buffer read performance");

	for exp in (0..10usize) {
		let nbytes = 1usize << (exp*3);
		let mut data = Vec::with_capacity(nbytes);
		data.resize(nbytes, 42u8);
		group.bench_with_input(BenchmarkId::new("BufferQueue::read", nbytes), &data, |b, data| {
			b.iter(|| {
				let mut q = BufferQueue::new();
				q.push(&data[..]);
				let mut nbytes = 0;
				let mut buf = [0u8; 1];
				loop {
					match q.read(&mut buf[..]) {
						Ok(v) if v == 1 => { nbytes += 1 },
						Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
							break
						},
						other => panic!("oh no! {:?}", other),
					}
				}
				assert_eq!(nbytes, data.len());
				nbytes
			});
		});
	}
} */

fn read_perf(c: &mut Criterion) {
	let mut group = c.benchmark_group("single-buffer read performance");
	let nbytes = 1usize << 20;
	let mut data = Vec::with_capacity(nbytes);
	data.resize(nbytes, 42u8);

	group.bench_function("BufferQueue", |b| {
		b.iter(|| {
			let mut q = BufferQueue::new();
			q.push(black_box(&data[..]));
			q.push_eof();
			let mut nbytes = 0;
			let mut buf = [0u8; 1];
			loop {
				match q.read(&mut buf[..]) {
					Ok(1) => nbytes += 1,
					Ok(0) => break,
					other => panic!("oh no! {:?}", other),
				}
			}
			assert_eq!(nbytes, data.len());
			nbytes
		});
	});

	group.bench_function("BufferQueue+BufReader", |b| {
		b.iter(|| {
			let mut q = BufferQueue::new();
			q.push(black_box(&data[..]));
			q.push_eof();
			let mut r = io::BufReader::new(q);
			let mut nbytes = 0;
			let mut buf = [0u8; 1];
			loop {
				match r.read(&mut buf[..]) {
					Ok(1) => nbytes += 1,
					Ok(0) => break,
					other => panic!("oh no! {:?}", other),
				}
			}
			assert_eq!(nbytes, data.len());
			nbytes
		});
	});

	group.bench_function("std::io::Read", |b| {
		b.iter(|| {
			let mut r = black_box(&data[..]);
			let mut nbytes = 0;
			let mut buf = [0u8; 1];
			loop {
				match r.read(&mut buf[..]) {
					Ok(1) => nbytes += 1,
					Ok(0) => break,
					other => panic!("oh no! {:?}", other),
				}
			}
			assert_eq!(nbytes, data.len());
			nbytes
		});
	});

	group.bench_function("Bytes", |b| {
		let src = Bytes::copy_from_slice(&data[..]);
		b.iter(|| {
			let mut r = black_box(src.clone()).reader();
			let mut nbytes = 0;
			let mut buf = [0u8; 1];
			loop {
				match r.read(&mut buf[..]) {
					Ok(1) => nbytes += 1,
					Ok(0) => break,
					other => panic!("oh no! {:?}", other),
				}
			}
			assert_eq!(nbytes, data.len());
			nbytes
		});
	});

	group.bench_function("std::io::Read+BufReader", |b| {
		b.iter(|| {
			let r = black_box(&data[..]);
			let mut r = io::BufReader::new(r);
			let mut nbytes = 0;
			let mut buf = [0u8; 1];
			loop {
				match r.read(&mut buf[..]) {
					Ok(1) => nbytes += 1,
					Ok(0) => break,
					other => panic!("oh no! {:?}", other),
				}
			}
			assert_eq!(nbytes, data.len());
			nbytes
		});
	});
}

criterion_group!(benches, read_perf);
criterion_main!(benches);
