use criterion::{criterion_group, criterion_main, Criterion};
use soroban_sdk::{contract, contractimpl, symbol_short, testutils::Address as _, Address, Env, Symbol, Vec};

const ACTIVE_PATH_LEN: usize = 5;

#[contract]
pub struct SwapPathBenchContract;

#[contractimpl]
impl SwapPathBenchContract {
    pub fn std_vec(env: Env, a0: Address, a1: Address, a2: Address, a3: Address, a4: Address) -> Symbol {
        let path = std::vec![a0, a1, a2, a3, a4];
        inspect_std_vec(&env, &path)
    }

    pub fn sdk_vec(env: Env, a0: Address, a1: Address, a2: Address, a3: Address, a4: Address) -> Symbol {
        let path = Vec::from_array(&env, [a0, a1, a2, a3, a4]);
        inspect_sdk_vec(&path)
    }

    pub fn fixed_array(
        env: Env,
        a0: Address,
        a1: Address,
        a2: Address,
        a3: Address,
        a4: Address,
    ) -> Symbol {
        let path = [Some(a0), Some(a1), Some(a2), Some(a3), Some(a4)];
        inspect_fixed_array(&env, &path)
    }
}

fn inspect_std_vec(_env: &Env, path: &[Address]) -> Symbol {
    if path.len() != ACTIVE_PATH_LEN {
        return symbol_short!("bad");
    }

    let mut count = 0usize;
    let mut first = None;
    let mut last = None;
    for hop in path {
        if count == 0 {
            first = Some(hop.clone());
        }
        last = Some(hop.clone());
        count += 1;
    }

    if first == last {
        symbol_short!("same")
    } else {
        symbol_short!("diff")
    }
}

fn inspect_sdk_vec(path: &Vec<Address>) -> Symbol {
    if path.len() as usize != ACTIVE_PATH_LEN {
        return symbol_short!("bad");
    }

    let mut count = 0u32;
    let mut first = None;
    let mut last = None;
    while count < path.len() {
        let hop = path.get(count).unwrap();
        if count == 0 {
            first = Some(hop.clone());
        }
        last = Some(hop);
        count += 1;
    }

    if first == last {
        symbol_short!("same")
    } else {
        symbol_short!("diff")
    }
}

fn inspect_fixed_array(env: &Env, path: &[Option<Address>; ACTIVE_PATH_LEN]) -> Symbol {
    let mut count = 0usize;
    let mut first = None;
    let mut last = None;
    for hop in path {
        match hop {
            Some(address) => {
                if count == 0 {
                    first = Some(address.clone());
                }
                last = Some(address.clone());
                count += 1;
            }
            None => break,
        }
    }

    if count != ACTIVE_PATH_LEN {
        return symbol_short!("bad");
    }

    if first == last {
        symbol_short!("same")
    } else {
        let _ = env;
        symbol_short!("diff")
    }
}

fn setup() -> (Env, SwapPathBenchContractClient<'static>, [Address; ACTIVE_PATH_LEN]) {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();

    let contract_id = env.register(SwapPathBenchContract, ());
    let client = SwapPathBenchContractClient::new(&env, &contract_id);
    let addrs = [
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    ];

    (env, client, addrs)
}

fn benchmark_cpu_costs() {
    let cases: [(&str, fn(&SwapPathBenchContractClient<'_>, &[Address; ACTIVE_PATH_LEN]) -> Symbol); 3] = [
        ("std::vec::Vec<Address>", |client, addrs| {
            client.std_vec(&addrs[0], &addrs[1], &addrs[2], &addrs[3], &addrs[4])
        }),
        ("soroban_sdk::Vec<Address>", |client, addrs| {
            client.sdk_vec(&addrs[0], &addrs[1], &addrs[2], &addrs[3], &addrs[4])
        }),
        ("[Option<Address>; 5]", |client, addrs| {
            client.fixed_array(&addrs[0], &addrs[1], &addrs[2], &addrs[3], &addrs[4])
        }),
    ];

    for (label, run_case) in cases {
        let (env, client, addrs) = setup();
        env.cost_estimate().budget().reset_default();
        let _ = run_case(&client, &addrs);
        let cpu = env.cost_estimate().budget().cpu_instruction_cost();
        let mem = env.cost_estimate().budget().memory_bytes_cost();
        eprintln!("[bench] {label:<28} cpu={cpu} mem={mem}");
    }
}

fn bench_swap_path(c: &mut Criterion) {
    let (env, client, addrs) = setup();
    let mut group = c.benchmark_group("swap_path");

    group.bench_function("std_vec", |b| {
        b.iter(|| client.std_vec(&addrs[0], &addrs[1], &addrs[2], &addrs[3], &addrs[4]))
    });
    group.bench_function("sdk_vec", |b| {
        b.iter(|| client.sdk_vec(&addrs[0], &addrs[1], &addrs[2], &addrs[3], &addrs[4]))
    });
    group.bench_function("fixed_array", |b| {
        b.iter(|| client.fixed_array(&addrs[0], &addrs[1], &addrs[2], &addrs[3], &addrs[4]))
    });

    let _ = env;
    benchmark_cpu_costs();
    group.finish();
}

criterion_group!(benches, bench_swap_path);
criterion_main!(benches);
