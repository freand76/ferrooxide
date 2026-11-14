// Processes start at time 0 as in the reference: top-level modules by name; in a scope its
// generate scopes first, then its instances (in source order, each with everything below it),
// then its own initial / always blocks. Port connections are in place before any of them.
module leaf(input [3:0] p); initial $display("%m leaf %b", p); endmodule
module mid; initial $display("%m mid a"); leaf l1(4'd1); leaf l2('1); initial $display("%m mid b"); endmodule
module zz; initial $display("top zz"); endmodule
module t;
  initial $display("t a");
  mid m1();
  initial $display("t b");
  leaf z1(4'd3);
  leaf a1(4'd4);
  genvar i;
  for (i = 0; i < 2; i = i + 1) begin : blk leaf q(i); end
  generate if (1) begin : g leaf lg(4'd9); initial $display("%m gen"); end endgenerate
  initial $display("t c");
endmodule
module aa; initial $display("top aa"); endmodule
