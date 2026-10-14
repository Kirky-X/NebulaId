// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! ID 防猜测混淆：4 轮 Feistel 网络上的 u64 双射编解码。
//! 可逆混淆非加密（不抗密钥穷举），仅防枚举/防规模推断；UUID 类超 u64 域须显式拒绝，不截断。

/// 轮数：4 轮足以打散连号局部性，编码/解码开销 O(1)。
const ROUNDS: usize = 4;

/// SplitMix64 终身常数，同时充当轮函数的乘法因子。
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// SplitMix64：把主密钥扩散成统计独立的轮密钥序列。
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(GOLDEN);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[derive(Debug, Clone)]
pub struct IdObfuscator {
    round_keys: [u64; ROUNDS],
}

impl IdObfuscator {
    /// 由部署密钥构建。同一部署所有实例必须持同一密钥，否则编码结果不一致。
    pub fn new(key: u64) -> Self {
        let mut state = key;
        let mut round_keys = [0u64; ROUNDS];
        for slot in round_keys.iter_mut() {
            *slot = splitmix64(&mut state);
        }
        Self { round_keys }
    }

    /// 轮函数：加性混入轮密钥后经黄金比例乘法扩散，取高 32 位作半域输出。
    fn round(&self, key: u64, half: u32) -> u32 {
        ((half as u64).wrapping_add(key).wrapping_mul(GOLDEN) >> 32) as u32
    }

    /// 混淆编码：u64 → u64 双射。连号输入的输出在空间上离散。
    pub fn encode(&self, id: u64) -> u64 {
        let (mut left, mut right) = ((id >> 32) as u32, id as u32);
        for (i, key) in self.round_keys.iter().enumerate() {
            let new_right = left ^ self.round(*key, right);
            left = right;
            right = new_right;
            let _ = i;
        }
        ((left as u64) << 32) | right as u64
    }

    /// 混淆解码：`encode` 的精确逆（轮密钥逆序应用）。
    pub fn decode(&self, encoded: u64) -> u64 {
        let (mut left, mut right) = ((encoded >> 32) as u32, encoded as u32);
        for key in self.round_keys.iter().rev() {
            let new_left = right ^ self.round(*key, left);
            right = left;
            left = new_left;
        }
        ((left as u64) << 32) | right as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obfuscator() -> IdObfuscator {
        IdObfuscator::new(0x0011_2233_4455_6677)
    }

    /// 确定性伪随机源（SplitMix64 序列），避免引入 rand 依赖。
    fn pseudo_random_seq(seed: u64, n: usize) -> Vec<u64> {
        let mut state = seed;
        (0..n).map(|_| splitmix64(&mut state)).collect()
    }

    /// 双射核心性质：随机输入 decode(encode(x)) == x。
    #[test]
    fn test_roundtrip_on_random_inputs() {
        let obf = obfuscator();
        for x in pseudo_random_seq(42, 10_000) {
            assert_eq!(obf.decode(obf.encode(x)), x, "roundtrip failed at {x}");
        }
    }

    /// 边界值：0、1、u64::MAX、半域边界必须恒等还原。
    #[test]
    fn test_roundtrip_on_boundaries() {
        let obf = obfuscator();
        for x in [0u64, 1, 2, u32::MAX as u64, (u32::MAX as u64) + 1, u64::MAX] {
            assert_eq!(
                obf.decode(obf.encode(x)),
                x,
                "boundary roundtrip failed at {x}"
            );
        }
    }

    /// 连号输入 0..1000 输出两两不同，且排序后无长度 ≥3 的公差等差段。
    #[test]
    fn test_consecutive_inputs_scatter_without_arithmetic_progressions() {
        let obf = obfuscator();
        let mut outputs: Vec<u64> = (0u64..1000).map(|x| obf.encode(x)).collect();

        let before = outputs.len();
        outputs.sort_unstable();
        outputs.dedup();
        assert_eq!(outputs.len(), before, "连号输入的输出不得重复");

        // 排序后 3-AP 检测：对每个中点 j 双向扫描 i<k，a[i]+a[k] == 2a[j] 即命中。
        // u128 承载求和：输出可逼近 u64::MAX，u64 加法会溢出 panic。
        for j in 1..outputs.len() - 1 {
            let mut i = j - 1;
            let mut k = j + 1;
            while i != usize::MAX && k < outputs.len() {
                let lhs = outputs[i] as u128 + outputs[k] as u128;
                let rhs = 2 * outputs[j] as u128;
                if lhs == rhs {
                    panic!(
                        "输出存在 3 项等差段: {} {} {}",
                        outputs[i], outputs[j], outputs[k]
                    );
                }
                if lhs < rhs {
                    k += 1;
                } else if i == 0 {
                    break;
                } else {
                    i -= 1;
                }
            }
        }
    }

    /// 不同密钥必须产生不同编码（同部署一致性 + 跨部署不互通的前提）。
    #[test]
    fn test_different_keys_produce_different_outputs() {
        let a = IdObfuscator::new(1);
        let b = IdObfuscator::new(2);
        let x = 0x0123_4567_89AB_CDEFu64;
        assert_ne!(a.encode(x), b.encode(x), "不同密钥不得产生相同编码");
        assert_eq!(a.decode(a.encode(x)), x);
        assert_eq!(b.decode(b.encode(x)), x);
    }

    /// 混淆不改变取值域：输出必须落在 u64 全域（无截断、无缩窄）。
    #[test]
    fn test_encode_is_total_over_u64() {
        let obf = obfuscator();
        // 全域性质由 roundtrip 测试覆盖；此处钉住 encode 在 u64 全域有定义。
        for x in pseudo_random_seq(7, 1_000) {
            let _ = obf.encode(x);
        }
    }
}
