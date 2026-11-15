interface bus_if #(parameter W = 4) (input logic clk);
  logic [W-1:0] data;
  logic valid;
  modport master (output data, valid, input clk);
  modport slave (input data, valid, clk);
endinterface

module prod (bus_if.master b);
  always_ff @(posedge b.clk) b.valid <= ~b.valid;
endmodule

module top;
  logic clk = 0;
  bus_if #(.W(8)) bi (clk);
  bus_if arr [2] (clk);
  prod p0 (.b(bi));
  prod p1 (.*);
  final $display("done");
endmodule
