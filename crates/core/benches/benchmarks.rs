use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::cell::Cell;

fn segment_generate_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("segment_algorithm");

    group.bench_function("generate_single", |b| {
        let current_id = 1000i64;
        let delta = 1i64;

        b.iter(|| {
            let mut id = current_id;
            id += delta;
            black_box(id)
        })
    });

    group.bench_function("generate_batch_10", |b| {
        let max_id = 2000i64;
        let delta = 1i64;
        let current_id = Cell::new(1000i64);

        b.iter(|| {
            let mut ids = Vec::with_capacity(10);
            for _ in 0..10 {
                let mut id = current_id.get();
                id += delta;
                current_id.set(id);
                if id >= max_id {
                    current_id.set(1000i64);
                }
                ids.push(black_box(id));
            }
            ids
        })
    });

    group.finish();
}

fn uuid_v7_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("uuid_v7");

    group.bench_function("generate_single", |b| {
        b.iter(|| {
            let uuid = uuid::Uuid::now_v7();
            black_box(uuid)
        })
    });

    group.finish();
}

criterion_group!(benches, segment_generate_benchmarks, uuid_v7_benchmarks);
criterion_main!(benches);
