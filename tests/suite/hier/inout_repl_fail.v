// exit: 1
// An inout port connects to a net, a select of one or a concatenation of them (IEEE
// 1800-2017 23.3.3), not to a replication.
module c(inout wire [7:0] w);
endmodule
module t;
  wire [3:0] bus;
  c u({2{bus}});
endmodule
