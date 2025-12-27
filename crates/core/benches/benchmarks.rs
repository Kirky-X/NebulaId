use criterion::{black_box, criterion_group, criterion_main, Criterion};
use nebula_core::algorithm::snowflake::SnowflakeAlgorithm;
use nebula_core::types::Id;
use std::cell::Cell;
use uuid::Uuid;

fn segment_generate_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("segment_algorithm");

    group.bench_function("generate_single", |b| {
        let current_id = 1000i64;
        let max_id = 2000i64;
        let step = 1000u32;
        let delta = 1u32;

        b.iter(|| {
            let mut id = current_id;
            id += delta;
            black_box(Id::from_i64(id))
        })
    });

    group.bench_function("generate_batch_10", |b| {
        let max_id = 2000i64;
        let step = 1000u32;
        let delta = 1u32;
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
                ids.push(black_box(Id::from_i64(id)));
            }
            ids
        })
    });

    group.bench_function("generate_batch_100", |b| {
        let max_id = 2000i64;
        let step = 1000u32;
        let delta = 1u32;
        let current_id = Cell::new(1000i64);

        b.iter(|| {
            let mut ids = Vec::with_capacity(100);
            for _ in 0..100 {
                let mut id = current_id.get();
                id += delta;
                current_id.set(id);
                if id >= max_id {
                    current_id.set(1000i64);
                }
                ids.push(black_box(Id::from_i64(id)));
            }
            ids
        })
    });

    group.bench_function("generate_batch_1000", |b| {
        let max_id = 2000i64;
        let step = 1000u32;
        let delta = 1u32;
        let current_id = Cell::new(1000i64);

        b.iter(|| {
            let mut ids = Vec::with_capacity(1000);
            for _ in 0..1000 {
                let mut id = current_id.get();
                id += delta;
                current_id.set(id);
                if id >= max_id {
                    current_id.set(1000i64);
                }
                ids.push(black_box(Id::from_i64(id)));
            }
            ids
        })
    });

    group.finish();
}

fn snowflake_generate_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("snowflake_algorithm");

    group.bench_function("generate_single", |b| {
        let algorithm = SnowflakeAlgorithm::new(1, 1);
        b.iter(|| {
            let id = algorithm.generate_id();
            black_box(id)
        })
    });

    group.bench_function("generate_batch_10", |b| {
        let algorithm = SnowflakeAlgorithm::new(1, 1);
        b.iter(|| {
            let mut ids = Vec::with_capacity(10);
            for _ in 0..10 {
                ids.push(black_box(algorithm.generate_id()));
            }
            ids
        })
    });

    group.bench_function("generate_batch_100", |b| {
        let algorithm = SnowflakeAlgorithm::new(1, 1);
        b.iter(|| {
            let mut ids = Vec::with_capacity(100);
            for _ in 0..100 {
                ids.push(black_box(algorithm.generate_id()));
            }
            ids
        })
    });

    group.bench_function("generate_batch_1000", |b| {
        let algorithm = SnowflakeAlgorithm::new(1, 1);
        b.iter(|| {
            let mut ids = Vec::with_capacity(1000);
            for _ in 0..1000 {
                ids.push(black_box(algorithm.generate_id()));
            }
            ids
        })
    });

    group.finish();
}

fn uuid_v7_generate_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("uuid_v7_algorithm");

    group.bench_function("generate_single", |b| {
        b.iter(|| {
            let uuid = Uuid::now_v7();
            black_box(Id::from_uuid_v7(uuid))
        })
    });

    group.bench_function("generate_batch_10", |b| {
        b.iter(|| {
            let mut ids = Vec::with_capacity(10);
            for _ in 0..10 {
                let uuid = Uuid::now_v7();
                ids.push(black_box(Id::from_uuid_v7(uuid)));
            }
            ids
        })
    });

    group.bench_function("generate_batch_100", |b| {
        b.iter(|| {
            let mut ids = Vec::with_capacity(100);
            for _ in 0..100 {
                let uuid = Uuid::now_v7();
                ids.push(black_box(Id::from_uuid_v7(uuid)));
            }
            ids
        })
    });

    group.bench_function("generate_batch_1000", |b| {
        b.iter(|| {
            let mut ids = Vec::with_capacity(1000);
            for _ in 0..1000 {
                let uuid = Uuid::now_v7();
                ids.push(black_box(Id::from_uuid_v7(uuid)));
            }
            ids
        })
    });

    group.finish();
}

fn id_conversion_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("id_conversion");

    let test_ids: Vec<Id> = (0..1000).map(|i| Id::from_i64(i)).collect();

    let idx = Cell::new(0usize);

    group.bench_function("to_string_numeric", |b| {
        b.iter(|| {
            let i = idx.get();
            idx.set((i + 1) % 1000);
            let id = &test_ids[i];
            black_box(id.to_string())
        })
    });

    group.bench_function("as_i64", |b| {
        b.iter(|| {
            let i = idx.get();
            idx.set((i + 1) % 1000);
            let id = &test_ids[i];
            black_box(id.as_i64())
        })
    });

    group.bench_function("as_u128", |b| {
        b.iter(|| {
            let i = idx.get();
            idx.set((i + 1) % 1000);
            let id = &test_ids[i];
            black_box(id.as_u128())
        })
    });

    group.finish();
}

fn concurrent_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_generation");

    group.bench_function("snowflake_concurrent_10", |b| {
        use std::sync::Arc;
        use std::thread;

        let algorithm = Arc::new(SnowflakeAlgorithm::new(1, 1));
        b.iter(|| {
            let mut handles = Vec::with_capacity(10);
            for _ in 0..10 {
                let algo = Arc::clone(&algorithm);
                handles.push(thread::spawn(move || algo.generate_id()));
            }
            let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
            black_box(results)
        })
    });

    group.bench_function("snowflake_concurrent_50", |b| {
        use std::sync::Arc;
        use std::thread;

        let algorithm = Arc::new(SnowflakeAlgorithm::new(1, 1));
        b.iter(|| {
            let mut handles = Vec::with_capacity(50);
            for _ in 0..50 {
                let algo = Arc::clone(&algorithm);
                handles.push(thread::spawn(move || algo.generate_id()));
            }
            let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
            black_box(results)
        })
    });

    group.finish();
}

fn throughput_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");

    group.bench_function("snowflake_10000_iterations", |b| {
        let algorithm = SnowflakeAlgorithm::new(1, 1);
        b.iter(|| {
            for _ in 0..10000 {
                black_box(algorithm.generate_id());
            }
        })
    });

    group.bench_function("uuid_v7_10000_iterations", |b| {
        b.iter(|| {
            for _ in 0..10000 {
                let uuid = Uuid::now_v7();
                black_box(Id::from_uuid_v7(uuid));
            }
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    segment_generate_benchmarks,
    snowflake_generate_benchmarks,
    uuid_v7_generate_benchmarks,
    id_conversion_benchmarks,
    concurrent_benchmarks,
    throughput_benchmarks
);
criterion_main!(benches);
