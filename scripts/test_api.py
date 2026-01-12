#!/usr/bin/env python3
"""
Nebula ID API 测试脚本
用法: python3 test_api.py [server_url]
默认 server_url = http://localhost:8080
"""

import requests
import json
import sys
import time
from datetime import datetime
from typing import Optional, Dict, Any


class NebulaAPITester:
    def __init__(self, server_url: str = "http://localhost:8080"):
        self.server_url = server_url.rstrip("/")
        self.passed = 0
        self.failed = 0
        self.results = []

    def log(self, message: str, level: str = "INFO"):
        """日志记录"""
        timestamp = datetime.now().strftime("%H:%M:%S")
        colors = {
            "INFO": "\033[94m",  # 蓝色
            "PASS": "\033[92m",  # 绿色
            "FAIL": "\033[91m",  # 红色
            "WARN": "\033[93m",  # 黄色
            "ENDC": "\033[0m",  # 重置
        }
        color = colors.get(level, colors["INFO"])
        print(f"{color}[{timestamp}] [{level}] {message}{colors['ENDC']}")

    def test_endpoint(
        self,
        method: str,
        endpoint: str,
        description: str,
        expected_status: int,
        api_key: Optional[str] = None,
        data: Optional[Dict] = None,
        params: Optional[Dict] = None,
    ) -> bool:
        """测试单个端点"""
        url = f"{self.server_url}{endpoint}"
        headers = {"Content-Type": "application/json"}
        if api_key:
            headers["Authorization"] = f"Bearer {api_key}"

        try:
            if method.upper() == "GET":
                response = requests.get(url, headers=headers, params=params, timeout=10)
            elif method.upper() == "POST":
                response = requests.post(url, headers=headers, json=data, timeout=10)
            elif method.upper() == "PUT":
                response = requests.put(url, headers=headers, json=data, timeout=10)
            elif method.upper() == "DELETE":
                response = requests.delete(url, headers=headers, timeout=10)
            else:
                raise ValueError(f"不支持的 HTTP 方法: {method}")

            actual_status = response.status_code
            success = actual_status == expected_status

            if success:
                self.passed += 1
                self.log(f"{description}: PASS (HTTP {actual_status})", "PASS")
            else:
                self.failed += 1
                self.log(
                    f"{description}: FAIL (Expected {expected_status}, got {actual_status})",
                    "FAIL",
                )

            # 记录结果
            self.results.append(
                {
                    "description": description,
                    "method": method.upper(),
                    "endpoint": endpoint,
                    "expected": expected_status,
                    "actual": actual_status,
                    "success": success,
                    "response": response.text[:200] if response.text else None,
                }
            )

            return success

        except requests.exceptions.RequestException as e:
            self.failed += 1
            self.log(f"{description}: FAIL (Request error: {str(e)})", "FAIL")
            return False

    def run_all_tests(self, admin_key: str = "test-admin-key"):
        """运行所有测试"""
        print("\n" + "=" * 60)
        print("🌌 Nebula ID API 测试套件 v1.0")
        print("=" * 60)
        print(f"🌐 测试服务器: {self.server_url}")
        print(f"📅 测试时间: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
        print("=" * 60 + "\n")

        # 1. 公共端点测试
        self.log("=" * 50, "INFO")
        self.log("1. 公共端点测试 (无需认证)", "INFO")
        self.log("=" * 50, "INFO")

        self.test_endpoint("GET", "/", "根路径健康检查", 200)
        self.test_endpoint("GET", "/health", "健康检查", 200)
        self.test_endpoint("GET", "/ready", "就绪检查", 200)
        self.test_endpoint("GET", "/metrics", "Prometheus 指标", 200)
        self.test_endpoint("GET", "/api/v1", "API 信息", 200)
        self.test_endpoint("GET", "/api-docs/openapi.json", "OpenAPI 文档", 200)
        self.test_endpoint("GET", "/nonexistent", "404 错误处理", 404)

        print()

        # 2. 认证测试
        self.log("=" * 50, "INFO")
        self.log("2. 认证测试", "INFO")
        self.log("=" * 50, "INFO")

        self.test_endpoint("POST", "/api/v1/generate", "未授权访问生成", 401)
        self.test_endpoint("POST", "/api/v1/generate/batch", "未授权访问批量生成", 401)
        self.test_endpoint("POST", "/api/v1/parse", "未授权访问解析", 401)
        self.test_endpoint("GET", "/api/v1/config", "未授权访问配置", 401)
        self.test_endpoint("GET", "/api/v1/workspaces", "未授权访问工作区", 401)

        print()

        # 3. ID 生成测试
        self.log("=" * 50, "INFO")
        self.log("3. ID 生成测试", "INFO")
        self.log("=" * 50, "INFO")

        # 测试无效请求
        self.test_endpoint("POST", "/api/v1/generate", "空请求体", 400, admin_key, {})
        self.test_endpoint(
            "POST",
            "/api/v1/generate",
            "缺少必需字段",
            400,
            admin_key,
            {"invalid": "data"},
        )

        # 实际 ID 生成测试 (假设有有效配置)
        result = self.test_endpoint(
            "POST",
            "/api/v1/generate",
            "生成 ID",
            200,
            admin_key,
            {"workspace": "test", "group": "test", "biz_tag": "test"},
        )
        if result:
            # 记录生成的 ID 用于后续测试
            pass

        # 批量生成
        self.test_endpoint(
            "POST",
            "/api/v1/generate/batch",
            "批量生成 ID",
            200,
            admin_key,
            {"workspace": "test", "group": "test", "size": 10},
        )

        print()

        # 4. 业务标签测试
        self.log("=" * 50, "INFO")
        self.log("4. 业务标签管理测试", "INFO")
        self.log("=" * 50, "INFO")

        self.test_endpoint(
            "POST",
            "/api/v1/biz-tags",
            "创建业务标签",
            201,
            admin_key,
            {
                "workspace_id": "test-ws",
                "name": "test-tag",
                "description": "Test tag created by API test",
            },
        )
        self.test_endpoint("GET", "/api/v1/biz-tags", "列出业务标签", 200, admin_key)
        self.test_endpoint(
            "POST",
            "/api/v1/biz-tags",
            "空标签名 (验证)",
            400,
            admin_key,
            {"workspace_id": "test", "name": "", "description": "Test"},
        )

        print()

        # 5. 工作区测试
        self.log("=" * 50, "INFO")
        self.log("5. 工作区管理测试", "INFO")
        self.log("=" * 50, "INFO")

        self.test_endpoint(
            "POST",
            "/api/v1/workspaces",
            "创建工作区",
            201,
            admin_key,
            {"name": "test-workspace", "description": "Test workspace"},
        )
        self.test_endpoint("GET", "/api/v1/workspaces", "列出工作区", 200, admin_key)
        self.test_endpoint(
            "GET", "/api/v1/workspaces/nonexistent", "获取不存在工作区", 404, admin_key
        )

        print()

        # 6. 组测试
        self.log("=" * 50, "INFO")
        self.log("6. 组管理测试", "INFO")
        self.log("=" * 50, "INFO")

        self.test_endpoint(
            "POST",
            "/api/v1/groups",
            "创建组",
            201,
            admin_key,
            {
                "workspace": "test-workspace",
                "name": "test-group",
                "description": "Test group",
            },
        )
        self.test_endpoint(
            "GET",
            "/api/v1/groups",
            "列出组",
            200,
            admin_key,
            params={"workspace": "test-workspace"},
        )

        print()

        # 7. API Key 测试
        self.log("=" * 50, "INFO")
        self.log("7. API Key 管理测试", "INFO")
        self.log("=" * 50, "INFO")

        self.test_endpoint(
            "POST",
            "/api/v1/api-keys",
            "创建 API Key",
            201,
            admin_key,
            {"name": "test-key", "role": "user", "workspace_id": "test-ws"},
        )
        self.test_endpoint("GET", "/api/v1/api-keys", "列出 API Keys", 200, admin_key)

        print()

        # 8. 配置测试
        self.log("=" * 50, "INFO")
        self.log("8. 配置管理测试", "INFO")
        self.log("=" * 50, "INFO")

        self.test_endpoint("GET", "/api/v1/config", "获取配置", 200, admin_key)
        self.test_endpoint(
            "POST",
            "/api/v1/config/rate-limit",
            "更新限速配置",
            200,
            admin_key,
            {"default_rps": 5000, "burst_size": 1000},
        )
        self.test_endpoint(
            "POST",
            "/api/v1/config/logging",
            "更新日志配置",
            200,
            admin_key,
            {"level": "debug"},
        )
        self.test_endpoint("POST", "/api/v1/config/reload", "重载配置", 200, admin_key)
        self.test_endpoint(
            "POST",
            "/api/v1/config/algorithm",
            "设置算法",
            200,
            admin_key,
            {"algorithm": "snowflake"},
        )

        print()

        # 9. 性能测试
        self.log("=" * 50, "INFO")
        self.log("9. 性能测试", "INFO")
        self.log("=" * 50, "INFO")

        # 吞吐量测试
        self.log("测试 ID 生成吞吐量 (50次)...", "INFO")
        start_time = time.time()
        success_count = 0
        for i in range(50):
            if self.test_endpoint(
                "POST",
                "/api/v1/generate",
                f"性能测试 {i + 1}/50",
                200,
                admin_key,
                {"workspace": "test", "group": "test", "biz_tag": "test"},
            ):
                success_count += 1
        end_time = time.time()
        duration = end_time - start_time
        rps = success_count / duration if duration > 0 else 0
        self.log(
            f"吞吐量测试完成: {success_count}/50 成功, {rps:.2f} req/s, 耗时 {duration:.2f}s",
            "WARN",
        )

        print()

        # 10. 并发测试
        self.log("=" * 50, "INFO")
        self.log("10. 并发测试", "INFO")
        self.log("=" * 50, "INFO")

        import concurrent.futures

        def concurrent_request():
            return self.test_endpoint(
                "POST",
                "/api/v1/generate",
                "并发请求",
                200,
                admin_key,
                {"workspace": "test", "group": "test", "biz_tag": "test"},
            )

        self.log("测试并发请求 (20 个并行请求)...", "INFO")
        start_time = time.time()
        with concurrent.futures.ThreadPoolExecutor(max_workers=20) as executor:
            futures = [executor.submit(concurrent_request) for _ in range(20)]
            results = [f.result() for f in concurrent.futures.as_completed(futures)]
        end_time = time.time()

        concurrent_success = sum(results)
        concurrent_duration = end_time - start_time
        self.log(
            f"并发测试完成: {concurrent_success}/20 成功, 耗时 {concurrent_duration:.2f}s",
            "WARN",
        )

        print()

        # 输出汇总
        self.print_summary()

    def print_summary(self):
        """打印测试汇总"""
        print("=" * 60)
        print("📊 测试结果汇总")
        print("=" * 60)
        print(f"  ✅ 通过: {self.passed}")
        print(f"  ❌ 失败: {self.failed}")
        print(f"  📈 总计: {self.passed + self.failed}")
        print()

        if self.failed == 0:
            print("🎉 所有测试通过！")
        else:
            print("⚠️  部分测试失败，以下是失败详情:")
            for r in self.results:
                if not r["success"]:
                    print(
                        f"  - {r['description']}: {r['method']} {r['endpoint']} (HTTP {r['actual']})"
                    )

        print("=" * 60)

        # 保存详细报告
        report_file = f"test_report_{datetime.now().strftime('%Y%m%d_%H%M%S')}.json"
        with open(report_file, "w", encoding="utf-8") as f:
            json.dump(
                {
                    "timestamp": datetime.now().isoformat(),
                    "server_url": self.server_url,
                    "passed": self.passed,
                    "failed": self.failed,
                    "total": self.passed + self.failed,
                    "results": self.results,
                },
                f,
                indent=2,
                ensure_ascii=False,
            )
        print(f"📄 详细报告已保存至: {report_file}")

        return self.failed == 0


if __name__ == "__main__":
    server_url = sys.argv[1] if len(sys.argv) > 1 else "http://localhost:8080"

    tester = NebulaAPITester(server_url)
    success = tester.run_all_tests()

    sys.exit(0 if success else 1)
