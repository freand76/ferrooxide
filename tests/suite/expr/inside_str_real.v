// `inside` (IEEE 1800-2017 11.4.13) compares integral values with ==? (x and z bits of a
// set value match anything) and strings and reals with ==.
module t;
  string s = "RO";
  real r = 2.5;
  logic [3:0] v = 4'b1010;
  function string get_access(); return s; endfunction
  localparam bit P = "WO" inside {"RW", "WO"};
  int bad = 0;
  initial begin
    if (!(s inside {"RO", "RC"})) bad++;
    if (get_access() inside {"RW", "WO"}) bad++;
    if (!(r inside {1.0, 2.5})) bad++;
    if (r inside {1.0, 3.0}) bad++;
    if (!(v inside {4'b1x1x})) bad++;
    if (v inside {4'b0x0x}) bad++;
    if (!P) bad++;
    if (bad == 0) $display("PASSED");
    else $display("FAILED: %0d", bad);
  end
endmodule
