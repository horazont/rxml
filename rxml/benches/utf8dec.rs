use criterion::{black_box, criterion_group, criterion_main, Criterion};

use rxml::lexer::{CodepointRead, DecodingReader, Utf8Decoder};

fn decode_perf_singlebyte(c: &mut Criterion) {
	let mut group = c.benchmark_group("single-buffer single-byte decode performance");
	let nbytes = 1usize << 20;
	let mut data = Vec::with_capacity(nbytes);
	data.resize(nbytes, b'x');

	group.bench_function("DecodingReader", |b| {
		b.iter(|| {
			let mut r = black_box(&data[..]);
			let mut dec = DecodingReader::new(&mut r);
			let mut n = 0;
			loop {
				match dec.read().unwrap() {
					None => break,
					Some(_) => n += 1,
				}
			}
			n
		});
	});

	group.bench_function("Utf8Decoder", |b| {
		b.iter(|| {
			let r = black_box(&data[..]);
			let mut dec = Utf8Decoder::new();
			let mut n = 0;
			for b in r {
				dec.feed(*b).unwrap().unwrap();
				n += 1;
			}
			n
		});
	});

	group.bench_function("stdlib+copy", |b| {
		b.iter(|| {
			let r = black_box(&data[..]);
			let s = std::str::from_utf8(r).unwrap();
			let mut n = 0;
			for _ in s.chars() {
				n += 1;
			}
			n
		});
	});
}

fn decode_perf_twobyte(c: &mut Criterion) {
	let mut group = c.benchmark_group("single-buffer two-byte decode performance");
	let nbytes = 1usize << 20;
	let mut data = Vec::with_capacity(nbytes);
	data.resize(nbytes, b'\x84');
	for i in 0..(data.len() / 2) {
		data[i * 2] = b'\xc3';
	}
	// this is a huge array of utf8(Ä) now.

	group.bench_function("DecodingReader", |b| {
		b.iter(|| {
			let mut r = black_box(&data[..]);
			let mut dec = DecodingReader::new(&mut r);
			let mut n = 0;
			loop {
				match dec.read().unwrap() {
					None => break,
					Some(_) => n += 1,
				}
			}
			n
		});
	});

	group.bench_function("Utf8Decoder", |b| {
		b.iter(|| {
			let r = black_box(&data[..]);
			let mut dec = Utf8Decoder::new();
			let mut n = 0;
			for b in r {
				let r = dec.feed(*b).unwrap();
				if n % 2 == 1 {
					r.unwrap();
					n += 1;
				}
			}
			n
		});
	});

	group.bench_function("stdlib+copy", |b| {
		b.iter(|| {
			let r = black_box(&data[..]);
			let s = std::str::from_utf8(r).unwrap();
			let mut n = 0;
			for _ in s.chars() {
				n += 1;
			}
			n
		});
	});
}

fn decode_perf_fourbyte(c: &mut Criterion) {
	let mut group = c.benchmark_group("single-buffer four-byte decode performance");
	let nbytes = 1usize << 20;
	let mut data = Vec::with_capacity(nbytes);
	data.resize(nbytes, 0);
	for i in 0..(data.len() / 4) {
		data[i * 4..(i + 1) * 4].copy_from_slice(&b"\xf0\x9f\x8e\x89"[..]);
	}
	// this is a huge array of utf8(Ä) now.

	group.bench_function("DecodingReader", |b| {
		b.iter(|| {
			let mut r = black_box(&data[..]);
			let mut dec = DecodingReader::new(&mut r);
			let mut n = 0;
			loop {
				match dec.read().unwrap() {
					None => break,
					Some(_) => n += 1,
				}
			}
			n
		});
	});

	group.bench_function("Utf8Decoder", |b| {
		b.iter(|| {
			let r = black_box(&data[..]);
			let mut dec = Utf8Decoder::new();
			let mut n = 0;
			for b in r {
				let r = dec.feed(*b).unwrap();
				if n % 4 == 3 {
					r.unwrap();
					n += 1;
				}
			}
			n
		});
	});

	group.bench_function("stdlib+copy", |b| {
		b.iter(|| {
			let r = black_box(&data[..]);
			let s = std::str::from_utf8(r).unwrap();
			let mut n = 0;
			for _ in s.chars() {
				n += 1;
			}
			n
		});
	});
}

criterion_group!(
	benches,
	decode_perf_singlebyte,
	decode_perf_twobyte,
	decode_perf_fourbyte
);
criterion_main!(benches);
