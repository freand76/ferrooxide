module m;
  function automatic integer acc(input integer value);
    begin : blk
      static int a = 1;
      automatic int b = 2;
      a = a + value + b;
      return a;
    end
  endfunction
  initial $display(acc(1));
endmodule
