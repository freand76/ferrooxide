module m;
  initial begin
    for (int i = 0; i < 2; i++)
      fork
        automatic int k = i;
        begin
          automatic int n = k;
          #1 $display(n);
        end
      join_none
    wait fork;
  end
endmodule
