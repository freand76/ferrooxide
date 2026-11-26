// expected
// global clocking (IEEE 1800-2017 14.14) and the global clocking sampled value functions
// (16.9.4): at tick j, a is seq[j] and b is seq[j+1] (the future value of a); a
// `$future_gclk` attempt of the last tick never ends
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  global clocking gc @(posedge clk); endclocking
  bit [8:0] seq = 9'b0_0110_1101;
  integer k = 0;
  always @(negedge clk) k <= k + 1;
  logic a, b;
  assign a = seq[k];
  assign b = seq[k + 1];

  int fut_p, fut_f, ris_p, ris_f, fal_p, fal_f, std_p, std_f, chg_p, chg_f, rose_p, rose_f, stb_p, stb_f, past_p, past_f;
  assert property (@(posedge clk) $future_gclk(a) === b) fut_p++; else fut_f++;
  assert property (@(posedge clk) $rising_gclk(a)) ris_p++; else ris_f++;
  assert property (@(posedge clk) $falling_gclk(a)) fal_p++; else fal_f++;
  assert property (@(posedge clk) $steady_gclk(a)) std_p++; else std_f++;
  assert property (@(posedge clk) $changing_gclk(a)) chg_p++; else chg_f++;
  assert property (@(posedge clk) $rose_gclk(a)) rose_p++; else rose_f++;
  assert property (@($global_clock) $stable_gclk(a)) stb_p++; else stb_f++;
  assert property (@(posedge clk) $past_gclk(b) === a) past_p++; else past_f++;

  initial #80 $finish;
  final begin
    $display("future %0d %0d", fut_p, fut_f);
    $display("rising %0d %0d", ris_p, ris_f);
    $display("falling %0d %0d", fal_p, fal_f);
    $display("steady %0d %0d", std_p, std_f);
    $display("changing %0d %0d", chg_p, chg_f);
    $display("rose %0d %0d", rose_p, rose_f);
    $display("stable %0d %0d", stb_p, stb_f);
    $display("past %0d %0d", past_p, past_f);
  end
endmodule
