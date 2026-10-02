# 비교 전용 — decide 런타임의 일부가 아니다. Rust `local::raw_logits`가 내는
# 원시 로짓을 Cloudflare/clef-flash 원본 PyTorch 구현(진짜 오라클)과 대조하는
# `crates/decide/tests/parity.rs`(`--features parity`)의 골든 fixture를
# 만든다. 실행에는 `transformers`/`torch`/`torchvision`/`Pillow`가 필요하다 —
# 저장소의 `src/decide/`(Laya 비교용 Python 패키지)와는 무관한 별도 venv를
# 쓴다:
#
#   python3 -m venv /tmp/clef-oracle-venv
#   /tmp/clef-oracle-venv/bin/pip install torch transformers torchvision Pillow \
#       safetensors huggingface_hub accelerate
#
# `joint_schema_model.py`는 Cloudflare/clef-flash HuggingFace 레포 자체
# 코드라 이 저장소에 커밋하지 않는다 — 실행 전에 아래 URL에서 받아
# `PYTHONPATH`에 둔다(CLEF_FLASH_MODEL_DIR가 가리키는 스냅샷 디렉터리에도
# 이미 포함되어 있다면 그 경로를 PYTHONPATH에 추가해도 된다):
#
#   curl -sL "https://huggingface.co/Cloudflare/clef-flash/raw/main/joint_schema_model.py" \
#       -o joint_schema_model.py
#
# 실행:
#   CLEF_FLASH_MODEL_DIR=<Cloudflare/clef-flash 로컬 스냅샷 경로> \
#   PYTHONPATH=<joint_schema_model.py가 있는 디렉터리> \
#   /tmp/clef-oracle-venv/bin/python scripts/clef_flash_oracle.py
import json
import os
import sys
from pathlib import Path

import torch

from joint_schema_model import load_release_model, encode_record, collate_records

GOLDEN_INPUTS = [
    {"state": "서버가 다운됐습니다", "questions": {"q": {"type": "noul", "instructions": "긴급한가?"}}},
    {
        "state": "중복 결제",
        "questions": {
            "q": {
                "type": "choice",
                "instructions": "어느 팀?",
                "criteria": {"billing": "billing", "technical": "technical"},
            }
        },
    },
    {
        "state": "결제가 11번 실패했습니다",
        "questions": {
            "q": {
                "type": "choice",
                "instructions": "어느 팀?",
                "criteria": {str(i): str(i) for i in range(11)},
            }
        },
    },
    {
        "state": "응답이 평소보다 느립니다",
        "questions": {
            "q": {
                "type": "score",
                "instructions": "심각도?",
                "criteria": ["낮음", "중간", "높음"],
            }
        },
    },
    {"state": "결제 시스템이 전부 마비됐습니다", "questions": {"q": {"type": "noul", "instructions": "긴급한가?"}}},
]


def main() -> None:
    model_dir = os.environ.get("CLEF_FLASH_MODEL_DIR", "Cloudflare/clef-flash")
    # Rust 쪽은 GGUF를 역양자화해 F32로, joint_head.safetensors도 F32로
    # 돌린다 — 오라클도 F32로 돌려야 "같은 정밀도에서 같은 가중치가 같은
    # 답을 내는가"를 비교할 수 있다. bf16으로 비교하면 백본 양자화 손실과
    # 오라클 자체의 bf16 라운딩이 섞여 어느 쪽이 원인인지 구분하기 어렵다.
    model, processor = load_release_model(model_dir, device="cpu", dtype=torch.float32)
    fixtures = []
    for case in GOLDEN_INPUTS:
        encoded = encode_record(processor.tokenizer, case, processor=processor)
        device = next(model.parameters()).device
        batch = collate_records([encoded], processor.tokenizer.pad_token_id, device)
        with torch.inference_mode():
            # model(batch) is list[list[Tensor]]: outer index is the batch
            # position, inner index is the question position within that
            # record. [0] selects the (only) batch item, giving
            # record_logits: list[Tensor] — one tensor of shape
            # (num_options,) per question. A second [0] here would wrongly
            # index into that tensor's single dimension instead of
            # selecting the "q" question (there's exactly one question per
            # golden input, so record_logits[0] IS the "q" tensor).
            record_logits = model(batch)[0]
        q_logits = record_logits[0].detach().cpu().tolist()
        fixtures.append({"input": case, "logits": [round(x, 6) for x in q_logits]})
        print(f"state={case['state']!r} logits={fixtures[-1]['logits']}", file=sys.stderr)
    out_dir = Path(__file__).resolve().parent.parent / "crates" / "decide" / "tests" / "parity_fixtures"
    out_dir.mkdir(parents=True, exist_ok=True)
    with open(out_dir / "golden.json", "w", encoding="utf-8") as f:
        json.dump(fixtures, f, ensure_ascii=False, indent=2)
    print(f"wrote {out_dir / 'golden.json'}", file=sys.stderr)


if __name__ == "__main__":
    main()
