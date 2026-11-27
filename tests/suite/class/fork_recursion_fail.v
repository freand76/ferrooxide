// exit: 1
// A function that calls itself from a branch of its own fork: each call would need branch
// processes of its own, which is not supported (reported, not compiled without end).
module t;
  int got;
  function automatic int twice(int k);
    if (k > 0)
      fork
        got = got + twice(k - 1);
      join_none
    return 2 * k;
  endfunction
  initial $display("%0d", twice(2));
endmodule
