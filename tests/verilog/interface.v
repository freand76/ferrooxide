// SystemVerilog interfaces: parameters, ports, modports, interface ports (with and without a
// modport), .* connections, signals read, written and waited on through the interface
interface bus_if #(parameter W = 8) (input logic clk);
  logic [W-1:0] data;
  logic         valid;
  logic         ready;
  modport master(output data, valid, input ready, clk);
  modport slave(input data, valid, clk, output ready);
  function automatic logic parity();
    return ^data;
  endfunction
  task automatic show(input [7:0] tag);
    $display("%0t: show %0d data %0d parity %b", $time, tag, data, parity());
  endtask
endinterface

module producer(bus_if.master b);
  logic [7:0] n = 0;
  always @(posedge b.clk) begin
    b.valid <= 1;
    if (b.ready) begin
      n <= n + 1;
      b.data <= n * 3;
    end
  end
endmodule

module consumer(bus_if.slave b, output logic [15:0] sum);
  initial sum = 0;
  assign b.ready = 1'b1;
  always @(posedge b.clk)
    if (b.valid) sum <= sum + b.data;
  always @(negedge b.clk)
    if (b.data == 9) b.show(7);
endmodule

module watcher(bus_if bus);
  always @(bus.data) $display("%0t: data %0d", $time, bus.data);
endmodule

module tb;
  logic clk = 0;
  logic [15:0] sum;
  always #5 clk = ~clk;
  bus_if #(.W(8)) bus(clk);
  producer p(.b(bus));
  consumer c(.b(bus), .sum(sum));
  watcher w(.*);
  side_if s();
  initial begin
    bus.data = 0;
    bus.valid = 0;
    s.flag = 1;
    #62 $display("sum %0d valid %b ready %b W %0d flag %b", sum, bus.valid, bus.ready, $bits(bus.data), s.flag);
    $finish;
  end
endmodule

interface side_if;
  logic flag;
endinterface
