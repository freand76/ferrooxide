// `release` in a clocked process that shares its event control with another one (they run
// as one group): releasing re-runs the net's continuous driver, not the releasing process.
module m;
  wire x;
  reg clk = 0;
  integer runs = 0, sub_runs = 0;
  assign x = 0;
  always #5 clk = ~clk;
  always @(posedge clk) begin
    runs = runs + 1;
    if (runs == 2) force x = 1;
    else if (runs == 4) release x;
  end
  s u(.clk(clk));
  initial begin
    #48;
    if (runs != 5 || sub_runs != 5 || x !== 0) $display("FAILED -- runs %0d %0d x %b", runs, sub_runs, x);
    $display("PASSED");
    $finish;
  end
endmodule
module s(input clk);
  always @(posedge clk) m.sub_runs = m.sub_runs + 1;
endmodule
