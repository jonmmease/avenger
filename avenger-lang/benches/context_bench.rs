use criterion::{black_box, criterion_group, criterion_main, Criterion};
use avenger_lang::context::EvaluationContext;

fn benchmark_evaluation_context_creation(c: &mut Criterion) {
    c.bench_function("create_evaluation_context", |b| {
        b.iter(|| {
            // Measure the time it takes to create a new EvaluationContext
            black_box(EvaluationContext::new())
        })
    });
}

criterion_group!(benches, benchmark_evaluation_context_creation);
criterion_main!(benches); 