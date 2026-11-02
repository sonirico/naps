#!/usr/bin/env bash
set -euo pipefail

trap 'kind delete cluster --name naps-e2e' EXIT

kind create cluster --name naps-e2e --wait 120s
kind load docker-image naps:dev --name naps-e2e

kubectl --context kind-naps-e2e apply -f e2e/nats.yaml
kubectl --context kind-naps-e2e rollout status deploy/nats --timeout=120s

helm install e2e charts/naps --kube-context kind-naps-e2e -f e2e/values-e2e.yaml \
  --set-file 'scripts.upper\.ts=e2e/upper.ts' --wait --timeout 120s

kubectl --context kind-naps-e2e run nats-box --image=natsio/nats-box:0.20.1 --restart=Never --command -- sleep 300
kubectl --context kind-naps-e2e wait --for=condition=Ready pod/nats-box --timeout=120s

kubectl --context kind-naps-e2e exec nats-box -- sh -c \
  'nats -s nats://nats.default.svc:4222 sub e2e.out --count 1 --raw > /tmp/out.txt 2>&1 &'

for _ in $(seq 1 30); do
  kubectl --context kind-naps-e2e exec nats-box -- nats -s nats://nats.default.svc:4222 pub e2e.in hello
  if kubectl --context kind-naps-e2e exec nats-box -- grep -q HELLO /tmp/out.txt; then
    echo "HELLO received on e2e.out"
    exit 0
  fi
  sleep 1
done

echo "HELLO never seen on e2e.out" >&2
exit 1
