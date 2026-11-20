// randcase (IEEE 1800-2017 18.16): an arm by its weight; weight 0 never; all 0: none.
module test;
  int n[3];
  int z = 0;
  int none = 1;
  initial begin
    repeat (3000)
      randcase
        1: n[0]++;
        z + 2: n[1]++;
        0: n[2]++;
      endcase
    randcase
      z: none = 0;
    endcase
    if (n[0] > 800 && n[0] < 1200 && n[1] > 1800 && n[1] < 2200 && n[2] == 0 && none)
      $display("PASSED");
    else
      $display("FAILED: %0d %0d %0d %0d", n[0], n[1], n[2], none);
  end
endmodule
