// A variable declared `automatic` in a static function is initialized at each call
// (IEEE 1800-2017 6.21); a static one keeps its value between calls.
module t;
  function int count(int step);
    automatic int fresh = 10;
    int kept = 0;
    fresh = fresh + step;
    kept = kept + step;
    return fresh * 100 + kept;
  endfunction
  initial begin
    if (count(1) !== 1101) $display("FAILED 1: %0d", count(1));
    else if (count(2) !== 1203) $display("FAILED 2");
    else if (count(3) !== 1306) $display("FAILED 3");
    else $display("PASSED");
  end
endmodule
