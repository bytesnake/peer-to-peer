cd ../../simulator && cargo build && cd ../libraries/example/;
LD_PRELOAD=../../simulator/target/debug/libpeersim.so target/release/peer_simulator simulate "$@"
